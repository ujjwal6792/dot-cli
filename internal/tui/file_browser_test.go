package tui

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"charm.land/lipgloss/v2"
	"github.com/charmbracelet/x/ansi"
)

func groupedStatusModel() model {
	m := largeStatusModel(0)
	for i := 0; i < 720; i++ {
		p := fmt.Sprintf(".agents/skills/skill-%03d/SKILL.md", i)
		m.dashboard.Managed = append(m.dashboard.Managed, DashboardEntry{Path: p, Dest: p, State: "unchanged"})
	}
	for _, path := range []string{".config/atuin/config.toml", ".config/yazi/yazi.toml", ".zshrc"} {
		m.dashboard.Managed = append(m.dashboard.Managed, DashboardEntry{Path: path, Dest: path, State: "changed"})
	}
	return m
}

func TestStatusGroupsTrackedDescendantsAndNavigatesOneDirectory(t *testing.T) {
	m := groupedStatusModel()
	rows := m.statusFiles()
	if len(rows) != 3 || rows[0].Path != ".agents" || rows[0].Tracked != 720 || rows[1].Path != ".config" || rows[1].Tracked != 2 {
		t.Fatalf("root should contain two folders and one file: %+v", rows)
	}
	current := send(m, "l").(model)
	if rows = current.statusFiles(); len(rows) != 1 || rows[0].Path != ".agents/skills" {
		t.Fatalf("opening .agents expanded the entire skill tree: %+v", rows)
	}
	current = send(current, "enter").(model)
	if len(current.statusFiles()) != 720 || current.browserDirs[TabStatus] != ".agents/skills" {
		t.Fatal("skill directories were not available after explicit navigation")
	}
	current = send(current, "h").(model)
	current = send(current, "h").(model)
	if current.browserDirs[TabStatus] != "" || current.statusFiles()[statusCursor(&current)].Path != ".agents" {
		t.Fatal("returning to HOME lost the directory focus")
	}
}

func TestStatusSearchReturnsToDirectoryAndPreviewStaysInline(t *testing.T) {
	m := groupedStatusModel()
	m.tabCursors[TabStatus] = 1
	m = send(m, "l").(model)
	m.search.SetValue("yazi.toml")
	m.searching = true
	m = send(m, "enter").(model)
	if got := m.statusFiles(); len(got) != 1 || got[0].Path != ".config/yazi/yazi.toml" {
		t.Fatalf("global tracked search lost full paths: %+v", got)
	}
	for _, key := range []string{"enter", "p"} {
		m = sendAndRun(m, key).(model)
		if m.stage != "returning-home" || m.loading {
			t.Fatal("file inspection took over the workspace")
		}
	}
	m = send(m, "esc").(model)
	if m.browserDirs[TabStatus] != ".config" || len(m.statusFiles()) != 2 {
		t.Fatal("clearing search lost the current tracked directory")
	}
}

func TestStatusSpaceExpandsAndCollapsesWithoutLosingBrowsingState(t *testing.T) {
	for _, width := range []int{40, 80, 140} {
		m := groupedStatusModel()
		m.width = width
		m.browserDirs[TabStatus] = ".config"
		m.search.SetValue("yazi")
		m.highlightPath = ".config/yazi/yazi.toml"
		m.highlight = HighlightResult{Status: "managed", Text: strings.Repeat("diff line\n", 60)}
		next, cmd := m.Update(key(" "))
		m = next.(model)
		if cmd != nil || !m.statusExpanded || m.previewViewport.Width() != width-4 {
			t.Fatal("Space must expand the existing preview without starting an operation")
		}
		view := m.View().Content
		if strings.Contains(ansi.Strip(view), "Tracked Files") || lipgloss.Width(view) > width || lipgloss.Height(view) > m.height {
			t.Fatal("expanded inspector should occupy the bounded workspace without the browser")
		}
		m = send(m, "J").(model)
		if m.previewViewport.YOffset() == 0 {
			t.Fatal("expanded diff cannot scroll")
		}
		m = send(m, " ").(model)
		if m.statusExpanded || m.browserDirs[TabStatus] != ".config" || m.search.Value() != "yazi" || m.statusFiles()[statusCursor(&m)].Path != ".config/yazi/yazi.toml" {
			t.Fatal("collapsing changed the selected file, directory, or filter")
		}
		m = send(m, " ").(model)
		m = send(m, "esc").(model)
		if m.statusExpanded || m.search.Value() != "yazi" {
			t.Fatal("Esc should collapse the inspector before clearing the filter")
		}
	}
}

func TestStatusSearchSpaceIsTextInsteadOfPreviewToggle(t *testing.T) {
	m := groupedStatusModel()
	m.searching = true
	m.search.Focus()
	m.search.SetValue("skill")
	m = send(m, " ").(model)
	if m.statusExpanded || m.search.Value() != "skill " {
		t.Fatal("Space in the search input must remain text")
	}
}

func trackedPrefs(path string) Entry {
	return Entry{Path: path, ID: path, Kind: "file", Status: "managed", OSStatus: "supported", TargetStatus: "unchanged", Metadata: map[string]any{"destination": path, "os": []string{"darwin", "linux"}, "portability": "managed-shared", "impact": "Safe"}}
}

func TestTrackShowsMixedChildrenAndOptionalTrackedFilterPreservesSelection(t *testing.T) {
	e := &fixtureEngine{inventory: Inventory{Entries: []Entry{
		trackedPrefs(".config/yazi/yazi.toml"), trackedPrefs(".config/atuin/config.toml"),
		{Path: ".config/private-app", Kind: "directory", Status: "eligible"},
		{Path: ".config/private-app/auth", Kind: "file", Status: "eligible"},
	}}}
	m := sendAndRun(bootReturning(t, e), "2").(model)
	if len(m.visible) != 1 || m.entries[m.visible[0]].Path != ".config" {
		t.Fatalf("partial tracked .config disappeared: %+v", m.visible)
	}
	m = sendAndRun(m, "l").(model)
	if len(m.visible) != 3 || !strings.Contains(ansi.Strip(m.View().Content), "untracked") {
		t.Fatal("Track must show untracked siblings by default")
	}
	m = send(m, "f").(model)
	if len(m.visible) != 2 {
		t.Fatal("optional tracked filter did not hide untracked siblings")
	}
	m = send(m, "h").(model)
	m = send(m, " ").(model)
	if len(m.basket) != 2 || m.basket[".config/private-app/auth"] != nil {
		t.Fatalf("tracked folder selected untracked descendants: %+v", m.basket)
	}
	m = sendAndRun(m, "l").(model)
	if len(m.visible) != 2 || m.entries[m.visible[0]].Path != ".config/atuin" {
		t.Fatal("tracked-only filter didn't show tracked child folders")
	}
	m = send(m, "f").(model)
	if len(m.visible) != 3 || len(m.basket) != 2 {
		t.Fatal("all-HOME discovery is unavailable or changed the basket")
	}
	m = send(m, "f").(model)
	m = send(m, "h").(model)
	m = sendAndRun(m, "3").(model)
	m = sendAndRun(m, "2").(model)
	if len(m.visible) != 1 || m.entries[m.visible[0]].Path != ".config" || e.applies != 0 {
		t.Fatal("tab caching lost the directory tree or navigation applied changes")
	}
}

type trackedBrowserFixture struct{ browserFixture }

func (f *trackedBrowserFixture) BrowserInventory(context.Context, string) (Inventory, error) {
	return Inventory{Entries: []Entry{trackedPrefs("config/prefs")}}, nil
}

func TestTrackLoadsUntrackedChildrenWhenFolderIsOpened(t *testing.T) {
	e := &trackedBrowserFixture{}
	m := sendAndRun(bootReturning(t, e), "2")
	if e.expansions != 0 {
		t.Fatal("opening Track descended into HOME folders")
	}
	m = sendAndRun(m, "l")
	if e.expansions != 1 || m.(model).browserDirs[TabTrack] != "config" {
		t.Fatal("opening a partly tracked folder didn't load its untracked children")
	}
	m = sendAndRun(m, "h")
	m = sendAndRun(m, "l")
	if e.expansions != 1 {
		t.Fatal("reopening a loaded folder repeated the directory read")
	}
}

func TestSpaceOpensUntrackedFileOptionsBeforeSelecting(t *testing.T) {
	e := &fixtureEngine{inventory: Inventory{Entries: []Entry{trackedPrefs("prefs")}}}
	e.inventory.Entries[0].Status = "eligible"
	m := sendAndRun(bootReturning(t, e), "2").(model)
	m = send(m, " ").(model)
	if m.stage != "metadata" || m.editPath != "prefs" || len(m.basket) != 0 {
		t.Fatal("Space should open tracking options without selecting/applying")
	}
	m = send(m, "esc").(model)
	if len(m.basket) != 0 {
		t.Fatal("canceling tracking options changed the basket")
	}
	m = send(m, " ").(model)
	m = send(m, "enter").(model)
	if len(m.basket) != 1 || e.plans != 0 || e.applies != 0 {
		t.Fatal("saving options must select the file and require separate plan review")
	}
	m = send(m, " ").(model)
	if len(m.basket) != 0 {
		t.Fatal("Space must still deselect a pending file")
	}
}

func folderOptionsModel(t *testing.T) (model, *fixtureEngine) {
	t.Helper()
	e := &fixtureEngine{inventory: Inventory{Entries: []Entry{
		{Path: "configs", Kind: "directory", Status: "eligible"},
		{Path: "configs/a", Kind: "file", Status: "eligible"},
		{Path: "configs/nested/b", Kind: "file", Status: "eligible"},
		{Path: "configs/auth", Kind: "file", Status: "restricted"},
	}}}
	m := sendAndRun(bootReturning(t, e), "2").(model)
	m = send(m, " ").(model)
	if m.stage != "metadata" || !m.editFolderSelection || len(m.basket) != 0 {
		t.Fatal("Space should open folder tracking options before taking a snapshot")
	}
	m.editValue["destination"] = "saved-configs"
	m.editValue["os"], m.editValue["portability"], m.editValue["impact"] = "darwin,linux", "managed-shared", "Safe"
	return m, e
}

func TestFolderOptionsFreezeExactEligiblePathsAndMapDestination(t *testing.T) {
	m, e := folderOptionsModel(t)
	m = sendAndRun(m, "enter").(model)
	if len(m.basket) != 2 || m.basket["configs/auth"] != nil || m.basket["configs/nested/b"]["destination"] != "saved-configs/nested/b" {
		t.Fatalf("folder options did not map exact eligible descendants: %+v", m.basket)
	}
	e.inventory.Entries = append(e.inventory.Entries, Entry{Path: "configs/later", Kind: "file", Status: "eligible"})
	if m.basket["configs/later"] != nil || e.applies != 0 || e.plans != 0 {
		t.Fatal("folder options implicitly selected future files or bypassed plan review")
	}
}

func TestLateFolderOptionsSnapshotCannotChangeAnotherTabBasket(t *testing.T) {
	m, _ := folderOptionsModel(t)
	next, snapshot := m.Update(key("enter"))
	next = sendAndRun(next, "3")
	next, _ = next.Update(snapshot())
	if got := next.(model); len(got.basket) != 0 || got.currentTab() != TabAdopt {
		t.Fatal("a late Track folder snapshot changed the Adopt basket")
	}
}

func TestFailedFolderLoadReturnsToRetryableParent(t *testing.T) {
	m := sendAndRun(NewReturningModel(&browserFixture{}), "2").(model)
	next, _ := m.Update(key("l"))
	m = next.(model)
	next, _ = m.Update(directoryMsg{generation: m.inventoryGeneration, path: "config", err: fmt.Errorf("permission denied")})
	m = next.(model)
	if m.browserDirs[TabTrack] != "" || m.current() < 0 || m.entries[m.current()].Path != "config" {
		t.Fatal("failed load left an empty directory without a retryable selection")
	}
	_, retry := m.Update(key("l"))
	if retry == nil || m.dirPending["config"] != true {
		t.Fatal("failed folder could not be retried")
	}
}

func TestTrackedBrowserFitsAndPreviewIsOnLeft(t *testing.T) {
	for _, size := range [][2]int{{40, 14}, {60, 18}, {80, 24}, {100, 32}, {140, 40}, {220, 55}} {
		m := groupedStatusModel()
		m.width, m.height = size[0], size[1]
		view := m.View().Content
		plain := ansi.Strip(view)
		if lipgloss.Width(view) > size[0] || lipgloss.Height(view) > size[1] {
			t.Fatalf("%v: workspace exceeds terminal: %dx%d", size, lipgloss.Width(view), lipgloss.Height(view))
		}
		if strings.Contains(plain, "skill-700/SKILL.md") || strings.Contains(plain, "Tracker Connection") || strings.Contains(plain, "Unified diff") {
			t.Fatal("root is cluttered by descendants or nested inspectors")
		}
		if size[0] >= 60 {
			var header string
			for _, line := range strings.Split(plain, "\n") {
				if strings.Contains(line, "Status & diff") {
					header = line
					break
				}
			}
			if strings.Index(header, "Status & diff") >= strings.Index(header, "Tracked Files") || m.browserPreviewWidth() > size[0]/3 {
				t.Fatalf("%v: inspector should be a narrow left column: %s", size, header)
			}
		}
	}
}

func TestDashboardOmitsOtherOSVariant(t *testing.T) {
	e, c := realNative(t)
	p := filepath.Join(c.Source, "manifest.json")
	var manifest map[string]any
	b, err := os.ReadFile(p)
	if err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal(b, &manifest); err != nil {
		t.Fatal(err)
	}
	entries := manifest["entries"].(map[string]any)
	for _, system := range []string{"darwin", "linux"} {
		entries["terminal_"+system] = map[string]any{"source": "home/prefs", "destination": ".config/terminal/config", "os": []string{system}, "portability": "managed-shared", "impact": "Safe"}
	}
	b, _ = json.Marshal(manifest)
	if err := os.WriteFile(p, b, 0600); err != nil {
		t.Fatal(err)
	}
	d, err := e.DashboardStatus(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if len(d.Managed) != 2 {
		t.Fatalf("dashboard included another OS's physical variant: %+v", d.Managed)
	}
}

func TestPreviewScrollUsesNarrowColumnGeometry(t *testing.T) {
	m := largeStatusModel(1)
	m.highlight = HighlightResult{Status: "managed", Text: strings.Repeat("diff line\n", 100) + "LAST DIFF LINE"}
	for i := 0; i < 110; i++ {
		m = send(m, "J").(model)
	}
	if !strings.Contains(ansi.Strip(m.View().Content), "LAST DIFF LINE") {
		t.Fatal("narrow preview cannot scroll to the final diff line")
	}
	if m.previewViewport.Width() != m.browserPreviewWidth()-4 {
		t.Fatal("scrolling still uses the retired wide inspector geometry")
	}
}

package tui

import (
	"context"
	"fmt"
	"strings"
	"testing"

	tea "charm.land/bubbletea/v2"
	"charm.land/lipgloss/v2"
	"github.com/charmbracelet/x/ansi"
)

type browserFixture struct {
	fixtureEngine
	fullLoads, rootLoads, expansions int
}

type inspectionFixture struct{ fixtureEngine }

func (f *inspectionFixture) Inspect(context.Context, string) (string, error) {
	return "old inspection", nil
}
func (f *inspectionFixture) Remote(context.Context, string, bool) error { return nil }

func TestLateInspectionCannotStealCurrentTab(t *testing.T) {
	m := largeStatusModel(2)
	m.engine = &inspectionFixture{}
	next, inspect := m.Update(key("p"))
	if inspect == nil {
		t.Fatal("missing inspection request")
	}
	next = send(next, "5")
	next, _ = next.Update(inspect())
	if got := next.(model); got.currentTab() != TabSync || got.stage != "returning-sync" {
		t.Fatal("late inspection replaced the current tab")
	}
}

func (f *browserFixture) InventoryMode(ctx context.Context, mode string) (Inventory, error) {
	f.fullLoads++
	return f.fixtureEngine.InventoryMode(ctx, mode)
}
func (f *browserFixture) BrowserInventory(context.Context, string) (Inventory, error) {
	f.rootLoads++
	return Inventory{Entries: []Entry{{Path: "config", Kind: "directory", Status: "eligible"}}}, nil
}
func (f *browserFixture) BrowseDirectory(context.Context, string) ([]Entry, error) {
	f.expansions++
	return []Entry{{Path: "config/prefs", Kind: "file", Status: "eligible"}}, nil
}

func TestTrackLoadsFoldersOnDemandAndSearchesAllOnlyOnEnter(t *testing.T) {
	e := &browserFixture{fixtureEngine: fixtureEngine{inventory: Inventory{Entries: []Entry{{Path: "config/prefs", Kind: "file", Status: "eligible"}}}}}
	m := NewModel(e, false, "track")
	m, _ = m.Update(m.Init()())
	if e.fullLoads != 0 || e.rootLoads != 1 {
		t.Fatal("opening Track performed a full scan")
	}
	m = sendAndRun(m, "l")
	if e.expansions != 1 || len(m.(model).entries) != 2 {
		t.Fatal("folder children did not load")
	}
	m = sendAndRun(m, "h")
	m = sendAndRun(m, "l")
	if e.expansions != 1 {
		t.Fatal("reopening a folder repeated I/O")
	}
	m = send(m, "/")
	m = send(m, "p")
	if e.fullLoads != 0 {
		t.Fatal("typing search triggered full scans")
	}
	m = sendAndRun(m, "enter")
	if e.fullLoads != 1 || len(m.(model).visible) != 1 {
		t.Fatal("explicit search did not find unloaded children")
	}
}

func TestLateDirectoryReplyCannotSwitchTabsOrLoseFocus(t *testing.T) {
	e := &browserFixture{}
	m := NewReturningModel(e)
	m = sendAndRun(m, "2")
	next, expand := m.Update(key("l"))
	m = send(next, "5")
	m, _ = m.Update(expand())
	if got := m.(model); got.currentTab() != TabSync || got.stage != "returning-sync" {
		t.Fatal("folder reply changed tabs")
	}
	m = sendAndRun(m, "2")
	_, retry := m.Update(key("l"))
	if retry == nil {
		t.Fatal("cancelled expansion stayed permanently pending")
	}
}

func largeStatusModel(n int) model {
	m := NewReturningModel(&fixtureEngine{}).(model)
	m.width, m.height = 100, 32
	m.dashboard = &DashboardData{}
	for i := 0; i < n; i++ {
		path := fmt.Sprintf("file-%05d.conf", i)
		m.dashboard.Managed = append(m.dashboard.Managed, DashboardEntry{Path: path, Dest: path, State: "changed"})
	}
	return m
}

func TestStatusScrollRendersActualSelectedFile(t *testing.T) {
	m := largeStatusModel(10000)
	m.tabCursors[TabStatus] = 9999
	view := ansiCSI.ReplaceAllString(m.View().Content, "")
	if !strings.Contains(view, "> file-09999.conf") || strings.Contains(view, "> file-00013.conf") {
		t.Fatalf("status list shows wrong data after scrolling:\n%s", view)
	}
}

func TestStatusFilterPersistsAfterEnter(t *testing.T) {
	m := largeStatusModel(100)
	m.search.SetValue("00042")
	m.searching = true
	next, _ := m.Update(key("enter"))
	got := next.(model)
	if files := got.statusFiles(); len(files) != 1 || files[0].Path != "file-00042.conf" {
		t.Fatalf("committing filter reset results: %d files", len(files))
	}
}

func TestStatusRejectsOutOfOrderPreview(t *testing.T) {
	m := largeStatusModel(100)
	first := m.statusHighlight()().(previewRequestMsg)
	_, work := m.startPreview(first)
	if work == nil {
		t.Fatal("first request did not start")
	}
	m.tabCursors[TabStatus] = 1
	second := m.statusHighlight()().(previewRequestMsg)
	if first.ctx.Err() == nil {
		t.Fatal("superseded request was not cancelled")
	}
	if _, cmd := m.startPreview(first); cmd != nil {
		t.Fatal("superseded debounce launched I/O")
	}
	fresh := HighlightResult{Status: "safe", Text: "newer file"}
	next, _ := m.Update(highlightMsg{second.generation, fresh, nil})
	next, _ = next.Update(highlightMsg{first.generation, HighlightResult{Status: "safe", Text: "stale file"}, nil})
	if got := next.(model); got.highlight.Text != "newer file" || got.highlightPath != "file-00001.conf" {
		t.Fatal("an old preview overwrote the selected file")
	}
}

func TestWorkspaceFitsTerminalWithLongPaths(t *testing.T) {
	for _, size := range [][2]int{{60, 18}, {80, 24}, {100, 32}, {140, 40}} {
		t.Run(fmt.Sprint(size), func(t *testing.T) {
			e := &fixtureEngine{inventory: Inventory{Entries: []Entry{}}}
			for i := 0; i < 500; i++ {
				e.inventory.Entries = append(e.inventory.Entries, Entry{Path: fmt.Sprintf("file-%04d-", i) + strings.Repeat("long", 30), Kind: "file", Status: "eligible"})
			}
			m := boot(t, e, false, "track")
			m, _ = m.Update(tea.WindowSizeMsg{Width: size[0], Height: size[1]})
			for i := 0; i < 60; i++ {
				m = send(m, "j")
			}
			view := m.View().Content
			if lipgloss.Width(view) > size[0] || lipgloss.Height(view) > size[1] {
				t.Fatalf("view %dx%d exceeds terminal %dx%d", lipgloss.Width(view), lipgloss.Height(view), size[0], size[1])
			}
			plain := ansiCSI.ReplaceAllString(view, "")
			if !strings.Contains(plain, "file-0060") || !strings.Contains(plain, "q quit") {
				t.Fatalf("focused row or footer disappeared:\n%s", plain)
			}
		})
	}
}

func TestReturningReviewKeepsGatesVisibleAndScrollsFullDiff(t *testing.T) {
	m := NewReturningModel(&fixtureEngine{}).(model)
	m.activeTab, m.stage = TabTrack, "review"
	m.selected = &PlanResult{Plan: map[string]any{"id": "plan-exact"}, Direction: "HOME → repository", GatesKnown: true, Unsafe: true, Desktop: true, Backup: true, Diffs: []Diff{{Path: "prefs", Text: strings.Repeat("context line\n", 100) + "LAST DIFF LINE"}}}
	m.refreshReviewViewport()
	view := ansiCSI.ReplaceAllString(m.View().Content, "")
	for _, want := range []string{"Unsafe", "Desktop", "Backup", "plan-exact", "a continue"} {
		if !strings.Contains(view, want) {
			t.Fatalf("review hid %q:\n%s", want, view)
		}
	}
	var current tea.Model = m
	for i := 0; i < 110; i++ {
		current = send(current, "j")
	}
	if !strings.Contains(current.View().Content, "LAST DIFF LINE") {
		t.Fatal("review diff cannot scroll to its last line")
	}
}

func TestSourceReplacementKeepsBothChoicesAndHintsVisible(t *testing.T) {
	for _, width := range []int{80, 100, 140} {
		m := NewModel(nil, false, "track").(model)
		m.width, m.height, m.stage = width, 32, "source-confirm"
		m.sourceBackup = "/private/tmp/long-acceptance-path/home/.local/share/dot-config.backup-20261016T075454.304844000Z/checkout"
		m.viewport.SetWidth(width - 6)
		plain := ansi.Strip(m.renderSource())
		for _, want := range []string{"Confirm Replace", "> Cancel", "Confirm backup and replace", "Enter confirm", "Esc back"} {
			if !strings.Contains(plain, want) {
				t.Fatalf("%d columns: missing %q:\n%s", width, want, plain)
			}
		}
		if lipgloss.Height(plain) > m.height {
			t.Fatalf("%d columns: replacement overflowed terminal", width)
		}
	}
}

package tui

import (
	"fmt"
	"strings"
	"testing"

	tea "charm.land/bubbletea/v2"
	"charm.land/lipgloss/v2"
	"github.com/charmbracelet/x/ansi"
)

func pointerClick(m model, x, y int) (model, tea.Cmd) {
	next, cmd := m.Update(tea.MouseClickMsg{X: x, Y: y, Button: tea.MouseLeft})
	return next.(model), cmd
}

func pointerText(t *testing.T, m model, text string) (int, int) {
	t.Helper()
	lines := strings.Split(ansi.Strip(m.View().Content), "\n")
	for y := len(lines) - 1; y >= 0; y-- {
		if i := strings.Index(lines[y], text); i >= 0 {
			return lipgloss.Width(lines[y][:i]), y
		}
	}
	t.Fatalf("missing pointer target %q in %s", text, strings.Join(lines, "\n"))
	return 0, 0
}

func TestMouseTabsWrapAndEditorsKeepTheirFocus(t *testing.T) {
	m := groupedStatusModel()
	m.width = 80
	if m.View().MouseMode != tea.MouseModeCellMotion {
		t.Fatal("terminal mouse reporting is disabled")
	}
	x, y := pointerText(t, m, "[6] Settings")
	if y != 2 {
		t.Fatal("fixture did not wrap the last tab")
	}
	m, _ = pointerClick(m, x+2, y)
	if m.currentTab() != TabSettings {
		t.Fatal("wrapped tab hitbox opened the wrong tab")
	}
	m.activeTab, m.stage = TabTrack, "metadata"
	m.editValue = map[string]any{}
	x, y = pointerText(t, m, "[1] Status")
	m, _ = pointerClick(m, x+2, y)
	if m.stage != "metadata" || m.currentTab() != TabTrack {
		t.Fatal("tab click discarded an unfinished editor")
	}
}

func TestMouseFocusDoubleClickAndFooterExpansionPreserveScope(t *testing.T) {
	m := groupedStatusModel()
	e := m.engine.(*fixtureEngine)
	preview, _, _ := m.browserColumns(true)
	y := lipgloss.Height(m.workspaceHeader()) + 3
	m, _ = pointerClick(m, preview+8, y) // .config, single focus only.
	if m.browserDirs[TabStatus] != "" || m.statusFiles()[statusCursor(&m)].Path != ".config" {
		t.Fatal("single click should focus a row without opening it")
	}
	m, _ = pointerClick(m, preview+8, y)
	if m.browserDirs[TabStatus] != ".config" || len(m.statusFiles()) != 2 {
		t.Fatal("double click did not open exactly one directory")
	}
	x, y := pointerText(t, m, "Space expand preview")
	m, _ = pointerClick(m, x+2, y)
	if !m.statusExpanded {
		t.Fatal("footer binding did not expand the inspector")
	}
	x, y = pointerText(t, m, "Space / Esc collapse")
	m, _ = pointerClick(m, x+2, y)
	if m.statusExpanded || m.browserDirs[TabStatus] != ".config" || len(m.basket) != 0 || e.applies != 0 || e.plans != 0 {
		t.Fatal("pointer browsing changed the location or authorized a write")
	}
}

func TestMouseCheckboxOptionsCanBeSavedAndCanceled(t *testing.T) {
	e := &fixtureEngine{inventory: Inventory{Entries: []Entry{{Path: "prefs", Kind: "file", Status: "eligible"}}}}
	m := sendAndRun(bootReturning(t, e), "2").(model)
	preview, _, _ := m.browserColumns(false)
	m, _ = pointerClick(m, preview+4, lipgloss.Height(m.workspaceHeader())+2)
	if m.stage != "metadata" || len(m.basket) != 0 {
		t.Fatal("checkbox click should open options before selecting an untracked file")
	}
	for _, field := range []int{1, 2, 3} {
		m, _ = pointerClick(m, 25, lipgloss.Height(m.workspaceHeader())+3+field)
	}
	x, y := pointerText(t, m, "Enter save")
	m, _ = pointerClick(m, x+2, y)
	if len(m.basket) != 1 || m.basket["prefs"]["impact"] != "Safe" || e.applies != 0 || e.plans != 0 {
		t.Fatal("saving mouse-selected options bypassed exact plan review")
	}
	m, _ = pointerClick(m, preview+4, lipgloss.Height(m.workspaceHeader())+2)
	if len(m.basket) != 0 {
		t.Fatal("checkbox click did not deselect the pending file")
	}
	m, _ = pointerClick(m, preview+4, lipgloss.Height(m.workspaceHeader())+2)
	x, y = pointerText(t, m, "Esc cancel")
	m, _ = pointerClick(m, x+2, y)
	if m.stage != "" || len(m.basket) != 0 {
		t.Fatal("canceling pointer-opened options changed the basket")
	}
}

func TestMouseFooterBreaksDoubleClickSequence(t *testing.T) {
	m := groupedStatusModel()
	x, y := m.browserPreviewWidth()+8, lipgloss.Height(m.workspaceHeader())+4
	m, _ = pointerClick(m, x, y) // Root .zshrc file.
	fx, fy := pointerText(t, m, "Space expand preview")
	m, _ = pointerClick(m, fx+2, fy)
	fx, fy = pointerText(t, m, "Space / Esc collapse")
	m, _ = pointerClick(m, fx+2, fy)
	m, _ = pointerClick(m, x, y)
	if m.statusExpanded {
		t.Fatal("intervening control clicks were treated as a file double-click")
	}
}

func TestMouseWheelTargetsPreviewListAndScrolledChildren(t *testing.T) {
	m := largeStatusModel(100)
	m.highlight = HighlightResult{Status: "managed", Text: strings.Repeat("diff line\n", 100)}
	m.resizePreview()
	next, _ := m.Update(tea.MouseWheelMsg{X: 3, Y: 10, Button: tea.MouseWheelDown})
	m = next.(model)
	if m.previewViewport.YOffset() == 0 || statusCursor(&m) != 0 {
		t.Fatal("preview scrolling moved the file selection")
	}
	next, _ = m.Update(tea.MouseWheelMsg{X: 50, Y: 10, Button: tea.MouseWheelDown})
	m = next.(model)
	if statusCursor(&m) != 3 || m.listOffsets[TabStatus] != 3 {
		t.Fatal("list wheel did not scroll the list and focus together")
	}
	x := m.browserPreviewWidth() + 8
	m, _ = pointerClick(m, x, lipgloss.Height(m.workspaceHeader())+2)
	if m.statusFiles()[statusCursor(&m)].Path != "file-00003.conf" {
		t.Fatal("click after scrolling selected an offscreen row")
	}
	m = largeStatusModel(0)
	m.width = 160
	for i := range 100 {
		m.dashboard.Managed = append(m.dashboard.Managed, DashboardEntry{Path: fmt.Sprintf("config/file-%03d", i)})
	}
	preview, list, child := m.browserColumns(true)
	if child == 0 {
		t.Fatal("fixture has no child preview")
	}
	next, _ = m.Update(tea.MouseWheelMsg{X: preview + list + 3, Y: 10, Button: tea.MouseWheelDown})
	m = next.(model)
	if m.childOffsets["config"] != 3 || m.browserDirs[TabStatus] != "" {
		t.Fatal("child scrolling navigated the parent list")
	}
	m, _ = pointerClick(m, preview+list+4, lipgloss.Height(m.workspaceHeader())+1)
	if m.browserDirs[TabStatus] != "config" || m.statusFiles()[statusCursor(&m)].Path != "config/file-003" {
		t.Fatal("click after child scrolling selected the wrong exact path")
	}
}

func TestMouseSettingsChecksDoNotInstallAndSearchCanFinish(t *testing.T) {
	e := &settingsFixture{}
	m := send(bootReturning(t, e), "6").(model)
	m.settingsSearch.Blur()
	_, anchors := m.settingsLayout()
	column := (m.width-2)/2 + 4
	m, cmd := pointerClick(m, column+4, lipgloss.Height(m.workspaceHeader())+3+anchors[1]+1)
	m = runCmds(m, cmd).(model)
	if e.verifies != 1 || e.installs != 0 {
		t.Fatal("settings pointer verification installed tools or missed the button")
	}
	m, _ = pointerClick(m, 8, lipgloss.Height(m.workspaceHeader())+1)
	if !m.settingsSearch.Focused() {
		t.Fatal("click did not focus settings search")
	}
	x, y := pointerText(t, m, "Enter finish")
	m, _ = pointerClick(m, x+2, y)
	if m.settingsSearch.Focused() {
		t.Fatal("clicking Enter finish did not finish search")
	}
}

func TestMouseConsentChoiceNeedsSeparateExplicitActivation(t *testing.T) {
	e := &fixtureEngine{}
	m := NewReturningModel(e).(model)
	m.activeTab, m.stage = TabAdopt, "confirm"
	m.width, m.height = 100, 32
	m.selected = &PlanResult{Plan: map[string]any{"id": "exact-pointer-plan"}, GatesKnown: true}
	x, y := pointerText(t, m, "Apply exact reviewed plan")
	m, cmd := pointerClick(m, x+3, y)
	if m.confirmChoice != 1 || cmd != nil || e.applies != 0 {
		t.Fatal("choosing Apply performed an operation without explicit activation")
	}
	x, y = pointerText(t, m, "Enter confirm")
	m, cmd = pointerClick(m, x+2, y)
	if cmd == nil || e.applies != 0 {
		t.Fatal("final pointer confirmation did not use the existing Apply command")
	}
	m = runCmds(m, cmd).(model)
	if e.applies != 1 {
		t.Fatal("separately confirmed pointer action did not apply the reviewed plan")
	}
	x, y = pointerText(t, m, "Enter return to Status")
	m, _ = pointerClick(m, x+2, y)
	if m.currentTab() != TabStatus || m.stage != "returning-home" || e.applies != 1 {
		t.Fatal("the completion screen did not return to Status without another Apply")
	}
}

func TestMouseReviewCheckboxesKeepAcknowledgmentsIndependent(t *testing.T) {
	m := NewReturningModel(&fixtureEngine{}).(model)
	m.activeTab, m.stage, m.width, m.height = TabAdopt, "review", 100, 32
	m.selected = &PlanResult{Plan: map[string]any{"id": "pointer-gates"}, GatesKnown: true, Unsafe: true, Desktop: true, Backup: true}
	m.planFresh = true
	x, y := pointerText(t, m, "[u] [ ]")
	m, _ = pointerClick(m, x+5, y)
	if !m.unsafeAck || m.desktopAck || m.backupAck || m.stage != "review" {
		t.Fatal("unsafe checkbox toggled another acknowledgment or advanced the plan")
	}
	x, y = pointerText(t, m, "[d] [ ]")
	m, _ = pointerClick(m, x+5, y)
	if !m.unsafeAck || !m.desktopAck || m.backupAck {
		t.Fatal("desktop checkbox did not remain independent")
	}
}

func TestSetupMouseChoiceAndContinueUseExistingWizard(t *testing.T) {
	m := NewInitModel(&fixtureEngine{}).(model)
	m.width, m.height, m.stage = 100, 32, "source-choice"
	x, y := pointerText(t, m, "New user: local git init")
	m, _ = pointerClick(m, x+3, y)
	if m.sourceChoice != 1 || m.loading {
		t.Fatal("setup choice click should only focus the chosen action")
	}
	x, y = pointerText(t, m, "Enter select")
	m, _ = pointerClick(m, x+2, y)
	if m.stage != "source-url" || m.sourceAction != "new" {
		t.Fatal("setup footer click did not open the expected URL input")
	}
}

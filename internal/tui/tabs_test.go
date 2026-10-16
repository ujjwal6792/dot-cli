package tui

import (
	"strings"
	"testing"

	tea "charm.land/bubbletea/v2"
)

type recoveryFixture struct {
	*fixtureEngine
	pending bool
}

func (r *recoveryFixture) RecoveryPending() (bool, error) { return r.pending, nil }

func bootReturning(t *testing.T, e Engine) tea.Model {
	t.Helper()
	m := NewReturningModel(e)
	m, _ = m.Update(tea.WindowSizeMsg{Width: 100, Height: 32})
	return m
}

func TestTabAndShiftTabCycleAllSixPages(t *testing.T) {
	m := bootReturning(t, &fixtureEngine{})
	for _, step := range []struct {
		mod  tea.KeyMod
		want int
	}{
		{0, 2}, {0, 3}, {0, 4}, {0, 5}, {0, 6}, {0, 1},
		{tea.ModShift, 6}, {tea.ModShift, 5}, {tea.ModShift, 4}, {tea.ModShift, 3}, {tea.ModShift, 2}, {tea.ModShift, 1},
	} {
		next, _ := m.Update(tea.KeyPressMsg{Code: tea.KeyTab, Mod: step.mod})
		m = next
		if got := m.(model).currentTab(); got != step.want {
			t.Fatalf("Tab modifier %v opened tab %d; want %d", step.mod, got, step.want)
		}
	}
}

func TestPageTabDoesNotRetargetAndMachineShortcutStaysInTrack(t *testing.T) {
	e := &fixtureEngine{inventory: Inventory{Machine: Machine{ID: "local_machine", OS: "darwin"}, Entries: []Entry{{Path: "prefs", Kind: "file", Status: "eligible", Metadata: map[string]any{"destination": "prefs", "os": []string{"darwin"}, "portability": "managed-shared", "impact": "Safe"}}}}}
	m := sendAndRun(bootReturning(t, e), "2")
	next, _ := m.Update(tea.KeyPressMsg{Code: tea.KeyTab})
	if got := next.(model); got.currentTab() != TabAdopt || len(got.basket) != 0 {
		t.Fatal("page navigation changed assignment intent")
	}
	m = sendAndRun(next, "2")
	m = send(m, "m")
	if got := m.(model); got.currentTab() != TabTrack || !strings.Contains(got.message, "local_machine only") {
		t.Fatal("machine shortcut did not retarget within Track")
	}
	if e.applies != 0 {
		t.Fatal("navigation or retargeting applied files")
	}
}

func TestTabInMetadataKeepsFormNavigation(t *testing.T) {
	m := NewReturningModel(&fixtureEngine{}).(model)
	m.activeTab, m.stage = TabTrack, "metadata"
	m.editValue = map[string]any{}
	next, _ := m.Update(tea.KeyPressMsg{Code: tea.KeyTab})
	got := next.(model)
	if got.currentTab() != TabTrack || got.stage != "metadata" || got.editField != 1 {
		t.Fatal("Tab escaped the metadata form")
	}
	next, _ = next.Update(tea.KeyPressMsg{Code: tea.KeyTab, Mod: tea.ModShift})
	if next.(model).editField != 0 {
		t.Fatal("Shift+Tab did not move to the previous form field")
	}
}

func TestReturningTabsSwitchAndRender(t *testing.T) {
	e := &fixtureEngine{inventory: Inventory{Entries: []Entry{{Path: "prefs", Kind: "file", Status: "eligible", Metadata: map[string]any{"destination": "prefs", "os": []string{"darwin"}, "portability": "managed-shared", "impact": "Safe"}}}}}
	m := bootReturning(t, e)
	if got := m.(model).currentTab(); got != TabStatus {
		t.Fatalf("default tab = %d, want Status", got)
	}
	view := m.View().Content
	for _, title := range []string{"Status & Diff", "Track", "Adopt", "Stop-Tracking", "Git Sync", "Settings"} {
		if !strings.Contains(view, title) {
			t.Fatalf("tab bar missing %q:\n%s", title, view)
		}
	}
	m = sendAndRun(m, "2")
	if got := m.(model); got.currentTab() != TabTrack || got.mode != "track" || len(got.entries) != 1 {
		t.Fatalf("Track tab did not load track inventory: tab=%d mode=%s entries=%d", got.currentTab(), got.mode, len(got.entries))
	}
	m = sendAndRun(m, "3")
	if got := m.(model); got.currentTab() != TabAdopt || got.mode != "adopt" {
		t.Fatalf("Adopt tab did not switch direction: %+v", got)
	}
	m = sendAndRun(m, "4")
	if got := m.(model); got.currentTab() != TabUntrack || got.mode != "untrack" {
		t.Fatalf("Stop-Tracking tab did not switch mode: %+v", got)
	}
	m = send(m, "5")
	if got := m.(model); got.currentTab() != TabSync || got.stage != "returning-sync" {
		t.Fatalf("Git Sync tab missing: %+v", got)
	}
	if !strings.Contains(m.View().Content, "GIT SYNC") {
		t.Fatalf("sync pane missing:\n%s", m.View().Content)
	}
	m = send(m, "W")
	if got := m.(model); got.currentTab() != TabSettings || got.stage != "returning-settings" {
		t.Fatalf("W did not open Settings: %+v", got)
	}
	view = m.View().Content
	for _, want := range []string{"immutable", "Dependencies", "Tracker", "Recovery"} {
		if !strings.Contains(view, want) {
			t.Fatalf("settings missing %q:\n%s", want, view)
		}
	}
	m = send(m, "A") // Compatibility details are deliberately disclosed on demand.
	m = send(m, "end")
	view = m.View().Content
	for _, want := range []string{"Scriptable parity", "Retained:"} {
		if !strings.Contains(view, want) {
			t.Fatalf("settings missing %q after scroll:\n%s", want, view)
		}
	}
	m = send(m, "1")
	if got := m.(model); got.currentTab() != TabStatus || got.stage != "returning-home" {
		t.Fatalf("1 did not return home: %+v", got)
	}
}

func TestReturningSelectionInvalidatesPlanAcks(t *testing.T) {
	e := &fixtureEngine{unsafe: true, inventory: Inventory{Entries: []Entry{{Path: "prefs", Kind: "file", Status: "eligible", Metadata: map[string]any{"destination": "prefs", "os": []string{"darwin"}, "portability": "managed-shared", "impact": "Safe"}}}}}
	m := bootReturning(t, e)
	m = sendAndRun(m, "2")
	m = send(m, " ")
	m = sendAndRun(m, "p")
	m = send(m, "u")
	if !m.(model).unsafeAck {
		t.Fatal("review gate ack setup failed")
	}
	m = sendAndRun(m, "2")
	m = send(m, " ")
	if got := m.(model); got.planFresh || got.unsafeAck {
		t.Fatalf("selection did not invalidate plan: fresh=%t unsafe=%t", got.planFresh, got.unsafeAck)
	}
}

func TestReturningRecoveryBlocksPullPush(t *testing.T) {
	e := &recoveryFixture{fixtureEngine: &fixtureEngine{}, pending: true}
	m := bootReturning(t, e)
	m = send(m, "5")
	m = send(m, "enter")
	got := m.(model)
	if got.stage == "returning-remote-confirm" {
		t.Fatal("recovery-pending pull reached confirmation")
	}
	if !strings.Contains(got.message, "Recovery pending") {
		t.Fatalf("missing recovery refusal: %q", got.message)
	}
	m = send(m, "P")
	got = m.(model)
	if got.stage == "returning-remote-confirm" || !strings.Contains(got.message, "Recovery pending") {
		t.Fatalf("recovery-pending push not refused: %+v", got)
	}
	view := m.View().Content
	if !strings.Contains(view, "Recovery:") || !strings.Contains(view, "blocked") {
		t.Fatalf("sync pane hides recovery state:\n%s", view)
	}
}

func TestReturningSettingsKeysStayViewOnly(t *testing.T) {
	e := &fixtureEngine{}
	m := bootReturning(t, e)
	m = send(m, "6")
	m = send(m, "D")
	if !strings.Contains(m.(model).message, "unavailable") {
		t.Fatalf("D without setup seam should say unavailable: %q", m.(model).message)
	}
	m = send(m, "R")
	if !m.(model).settingsArmed {
		t.Fatal("first R did not arm backup-first replace")
	}
	m = send(m, "esc")
	if got := m.(model); got.settingsArmed || got.stage != "returning-home" {
		t.Fatalf("Esc did not cancel arming and go back: %+v", got)
	}
}

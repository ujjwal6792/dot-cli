package tui

import (
	"context"
	"fmt"
	"strings"
	"testing"

	tea "charm.land/bubbletea/v2"
	"charm.land/lipgloss/v2"
)

type settingsFixture struct {
	fixtureEngine
	checks, verifies, installs int
}

func (f *settingsFixture) ConfigPaths() (string, string, string, string) {
	return "/home/test/.local/share/dot-config", "/home/test", "/home/test/.local/state/dot", "/home/test/.config/chezmoi/chezmoi.toml"
}
func (f *settingsFixture) Check(context.Context) (string, error) { f.checks++; return "old-user", nil }
func (f *settingsFixture) InitDependencies(context.Context) (DependencyResult, error) {
	f.verifies++
	return DependencyResult{Ready: true, Rows: []DependencyRow{{Tool: "git", Version: "2.49.0"}, {Tool: "chezmoi", Version: "v2.73.1"}}}, nil
}
func (f *settingsFixture) InstallDependencies(context.Context, map[string]any, string, TerminalStreams) error {
	f.installs++
	return fmt.Errorf("unexpected installation")
}
func (f *settingsFixture) InitTracker(context.Context, map[string]any) (SetupResult, error) {
	return SetupResult{}, nil
}
func (f *settingsFixture) Complete(context.Context) error { return nil }

func TestSettingsSearchDoesNotExecuteShortcuts(t *testing.T) {
	e := &settingsFixture{}
	m := send(bootReturning(t, e), "6")
	m = send(m, "/")
	for _, s := range []string{"D", "q", "6", "?"} {
		m = send(m, s)
	}
	got := m.(model)
	if got.settingsSearch.Value() != "Dq6?" || got.stage != "returning-settings" || got.help || e.verifies != 0 {
		t.Fatal("search text triggered an application action")
	}
	m = send(m, "esc")
	if got := m.(model); got.stage != "returning-settings" || got.settingsSearch.Value() != "" {
		t.Fatal("Esc did not clear and leave search")
	}
}

func TestSettingsButtonsActivateAndStayVisibleOnNarrowTerminal(t *testing.T) {
	e := &settingsFixture{}
	m := send(bootReturning(t, e), "6")
	m, _ = m.Update(tea.WindowSizeMsg{Width: 80, Height: 24})
	m = send(m, "ctrl+n")
	if view := safeTerminal(m.View().Content); !strings.Contains(view, "[D] Verify tools") {
		t.Fatalf("focused button hidden:\n%s", view)
	}
	m = sendAndRun(m, "enter")
	if e.verifies != 1 || e.installs != 0 {
		t.Fatal("verify button did not perform a read-only check")
	}
	m = send(m, "ctrl+n")
	if view := safeTerminal(m.View().Content); !strings.Contains(view, "[T] Check tracker") {
		t.Fatalf("tracker button hidden:\n%s", view)
	}
	m = sendAndRun(m, "enter")
	if e.checks != 1 {
		t.Fatal("tracker button did not run its check")
	}
	m = send(m, "ctrl+n")
	m = send(m, "enter")
	if !m.(model).settingsArmed || e.installs != 0 {
		t.Fatal("backup action did not preserve the separate confirmation")
	}
}

func TestSettingsFilterPersistsAndFocusSkipsHiddenActions(t *testing.T) {
	e := &settingsFixture{}
	m := send(bootReturning(t, e), "6")
	m = send(m, "/")
	for _, r := range "dependencies" {
		m = send(m, string(r))
	}
	m = send(m, "enter")
	view := safeTerminal(m.View().Content)
	if strings.Contains(view, "Paths · read-only") || !strings.Contains(view, "Dependencies") {
		t.Fatalf("filter did not narrow sections:\n%s", view)
	}
	m = send(m, "ctrl+n")
	m = sendAndRun(m, "enter")
	if e.verifies != 1 || e.checks != 0 {
		t.Fatal("Control navigation selected a hidden action")
	}
}

func TestSettingsAndPaddedFooterFitTerminal(t *testing.T) {
	for _, size := range [][2]int{{60, 18}, {80, 24}, {100, 32}, {140, 40}} {
		t.Run(fmt.Sprint(size), func(t *testing.T) {
			m := send(bootReturning(t, &settingsFixture{}), "6")
			m, _ = m.Update(tea.WindowSizeMsg{Width: size[0], Height: size[1]})
			for _, action := range []string{"ctrl+n", "ctrl+n", "ctrl+n", "ctrl+n"} {
				m = send(m, action)
				view := m.View().Content
				if lipgloss.Width(view) > size[0] || lipgloss.Height(view) > size[1] {
					t.Fatalf("settings exceeds terminal: %dx%d", lipgloss.Width(view), lipgloss.Height(view))
				}
			}
			footer := strings.Split(safeTerminal(m.(model).renderFooter("q quit\nEsc back")), "\n")
			if len(footer) != 6 || strings.TrimSpace(footer[1]) != "" {
				t.Fatal("footer lost its reserved top padding")
			}
		})
	}
}

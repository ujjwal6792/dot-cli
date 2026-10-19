package tui

import (
	"strings"
	"testing"

	"charm.land/lipgloss/v2"
	"github.com/charmbracelet/x/ansi"
)

func TestFooterWrapsBindingsOnlyWhenTerminalWidthRequiresIt(t *testing.T) {
	m := NewReturningModel(&fixtureEngine{}).(model)
	keys := "q quit · Tab/Shift+Tab tabs\nSpace select · / search"
	m.width = 70
	rows := strings.Split(ansi.Strip(m.renderFooter(keys)), "\n")
	if !strings.Contains(rows[4], "q quit · Tab/Shift+Tab tabs · Space select · / search") || strings.TrimSpace(rows[5]) != "" {
		t.Fatal("footer kept an early hardcoded line break despite available width")
	}
	m.width = 40
	view := m.renderFooter(keys)
	rows = strings.Split(ansi.Strip(view), "\n")
	if !strings.Contains(rows[4], "Tab/Shift+Tab tabs") || !strings.Contains(rows[5], "Space select · / search") || lipgloss.Width(view) > 40 || len(rows) != 6 {
		t.Fatal("narrow footer split a binding or lost the fixed footer geometry")
	}
	view = ansi.Strip(m.renderFooter("Space expand preview · Tab/Shift+Tab tabs · j/k move · h/l folders · J/K preview · r rescan · q quit · ? help"))
	if !strings.Contains(view, "q quit") || !strings.Contains(view, "? help") || !strings.Contains(view, "…") {
		t.Fatal("an overflowing footer should retain quit and help")
	}
	m.width = 80
	view = ansi.Strip(m.workspaceFooter(true))
	if !strings.Contains(view, "q quit") || !strings.Contains(view, "? help") {
		t.Fatal("packing the Status footer removed quit or help")
	}
}

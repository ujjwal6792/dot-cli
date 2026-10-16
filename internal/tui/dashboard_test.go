package tui

import (
	"strings"
	"testing"
)

// Tables must render body rows, not just headers. bubbles table blanks its
// rows when its internal viewport has no explicit width; these tests pin the
// regression: header-only output is a failure.
func TestDependencyTableRendersRows(t *testing.T) {
	out := ansiCSI.ReplaceAllString(dependencyTable(DependencyResult{Ready: true, Rows: []DependencyRow{
		{Tool: "git", Link: "https://git-scm.com/", Version: "2.49.0"},
		{Tool: "chezmoi", Link: "https://www.chezmoi.io/", Version: "v2.73.1"},
	}}, false), "")
	for _, want := range []string{"PACKAGE", "git", "2.49.0", "chezmoi", "v2.73.1", "supported"} {
		if !strings.Contains(out, want) {
			t.Fatalf("table missing %q:\n%s", want, out)
		}
	}
}

func TestDashboardTableRendersRows(t *testing.T) {
	out := ansiCSI.ReplaceAllString(dashboardTable([]DashboardEntry{
		{Path: ".bashrc", Dest: ".bashrc", State: "unchanged"},
		{Path: ".vimrc", Dest: ".vimrc", State: "changed"},
	}), "")
	for _, want := range []string{"FILE", ".bashrc", ".vimrc", "current", "changed"} {
		if !strings.Contains(out, want) {
			t.Fatalf("table missing %q:\n%s", want, out)
		}
	}
}

func TestDashboardEmptyStateGuidesToTracking(t *testing.T) {
	m := model{initMode: false, returning: true, stage: "returning-home", width: 80, height: 30, dashboard: &DashboardData{}}
	view := ansiCSI.ReplaceAllString(m.View().Content, "")
	if !strings.Contains(view, "Nothing tracked yet") {
		t.Fatalf("missing empty guidance:\n%s", view)
	}
}

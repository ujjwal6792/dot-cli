package tui

import (
	"strings"

	"charm.land/lipgloss/v2"
)

// Footer geometry is shared by every workspace: separator, top padding,
// two status rows, then two keymap rows. Reserve it before sizing content.
func (m model) renderFooter(keys string) string {
	w := max(1, m.width)
	inner := max(1, w-2)
	statusColor := AppTheme.Muted
	switch {
	case m.err != nil || m.dashboardErr != nil && m.currentTab() == TabStatus:
		statusColor = AppTheme.Danger
	case m.loading || m.dashboardLoading || m.settingsArmed:
		statusColor = AppTheme.Status
	case strings.Contains(strings.ToLower(m.message), "failed") || strings.Contains(strings.ToLower(m.message), "unavailable"):
		statusColor = AppTheme.Danger
	case strings.Contains(strings.ToLower(m.message), "verified"):
		statusColor = AppTheme.Success
	case len(m.basket) > 0:
		statusColor = AppTheme.Success
	}
	msg := lipgloss.NewStyle().Width(inner).Foreground(lipgloss.Color(statusColor)).Render(m.workspaceMessage())
	rows := strings.Split(region(msg, inner, 2), "\n")
	keyStyle := lipgloss.NewStyle().Foreground(lipgloss.Color(AppTheme.Status)).Bold(true)
	labelStyle := lipgloss.NewStyle().Foreground(lipgloss.Color(AppTheme.Muted))
	for _, line := range strings.Split(region(keys, inner, 2), "\n") {
		parts := strings.Split(strings.TrimRight(line, " "), " · ")
		for i, part := range parts {
			key, label, ok := strings.Cut(part, " ")
			if ok {
				parts[i] = keyStyle.Render(key) + labelStyle.Render(" "+label)
			} else {
				parts[i] = labelStyle.Render(part)
			}
		}
		rows = append(rows, cellLine(strings.Join(parts, labelStyle.Render(" · ")), inner))
	}
	for i := range rows {
		rows[i] = " " + rows[i] + " "
	}
	separator := lipgloss.NewStyle().Foreground(lipgloss.Color(AppTheme.Accent)).Render(strings.Repeat("─", w))
	return surface(separator+"\n\n"+strings.Join(rows, "\n"), w, 6, AppTheme.Mantle)
}

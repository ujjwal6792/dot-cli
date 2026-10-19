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
	for _, line := range footerKeyLines(keys, inner) {
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

// Existing line breaks group hints at their call sites; actual terminal width
// decides where a new keymap row begins. Keep each binding intact.
func footerKeyLines(keys string, width int) []string {
	var lines []string
	line := ""
	for _, part := range strings.Split(strings.ReplaceAll(keys, "\n", " · "), " · ") {
		part = strings.TrimSpace(part)
		if part == "" {
			continue
		}
		candidate := part
		if line != "" {
			candidate = line + " · " + part
		}
		if line != "" && lipgloss.Width(candidate) > width {
			lines = append(lines, line)
			line = part
		} else {
			line = candidate
		}
	}
	if line != "" {
		lines = append(lines, line)
	}
	if len(lines) > 2 {
		suffix := "…"
		if strings.Contains(keys, "q quit") && !strings.Contains(lines[0], "q quit") {
			suffix += " · q quit"
		}
		if strings.Contains(keys, "? help") && !strings.Contains(lines[0], "? help") {
			suffix += " · ? help"
		}
		var parts []string
		for _, part := range strings.Split(lines[1], " · ") {
			if strings.HasPrefix(part, "q quit") && strings.Contains(suffix, "q quit") || part == "? help" && strings.Contains(suffix, "? help") {
				continue
			}
			parts = append(parts, part)
		}
		for len(parts) > 0 && lipgloss.Width(strings.Join(parts, " · ")+" · "+suffix) > width {
			parts = parts[:len(parts)-1]
		}
		parts = append(parts, suffix)
		lines[1] = strings.Join(parts, " · ")
	}
	return strings.Split(region(strings.Join(lines, "\n"), width, 2), "\n")
}

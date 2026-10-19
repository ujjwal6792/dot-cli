package tui

import (
	"strings"

	tea "charm.land/bubbletea/v2"
	"charm.land/lipgloss/v2"
)

func (m model) mouseMetadata(event tea.MouseClickMsg, lines []string) (tea.Model, tea.Cmd) {
	top := lipgloss.Height(m.workspaceHeader())
	if event.Y <= top || event.Y >= top+m.workspaceHeight()-1 {
		return m, nil
	}
	fields := []string{"destination", "os", "portability", "impact", "template", "git_keys", "removals", "tools", "warnings", "integrations"}
	limit := max(3, min(len(fields), m.workspaceHeight()-9))
	start := max(0, min(m.editField-2, len(fields)-limit))
	row := event.Y - lipgloss.Height(m.workspaceHeader()) - 3
	if row >= 0 && row < limit {
		m.editField = start + row
		m.message = ""
		if _, choices := metadataChoices[fields[m.editField]]; choices && event.X >= 20 {
			return m.mouseKey("Space")
		}
		return m, nil
	}
	if event.Y < lipgloss.Height(m.workspaceHeader())+3+limit || event.Y >= len(lines)-6 {
		return m, nil
	}
	// Guidance lists the valid values. Prefer the longest matching value so
	// darwin,linux remains one choice instead of matching its darwin prefix.
	choices := metadataChoices[fields[m.editField]]
	value, longest := "", 0
	for _, choice := range choices {
		label := choice
		if label == "" {
			label = "(empty)"
		}
		for from := 0; from < len(lines[event.Y]); {
			i := strings.Index(lines[event.Y][from:], label)
			if i < 0 {
				break
			}
			i += from
			x := lipgloss.Width(lines[event.Y][:i])
			if event.X >= x && event.X < x+lipgloss.Width(label) && len(label) > longest {
				value, longest = choice, len(label)
			}
			from = i + len(label)
		}
	}
	if longest > 0 {
		m.editValue[fields[m.editField]] = value
		m.message = ""
	}
	return m, nil
}

func (m model) mouseSettings(event tea.MouseClickMsg) (tea.Model, tea.Cmd) {
	top := lipgloss.Height(m.workspaceHeader())
	if event.Y >= top && event.Y < top+3 {
		cmd := m.focusSetting(0)
		return m, cmd
	}
	if event.Y < top+3 || event.Y >= top+m.workspaceHeight() {
		return m, nil
	}
	_, anchors := m.settingsLayout()
	for action, y := range anchors {
		buttonY := top + 3 + y - m.viewport.YOffset()
		if event.Y < buttonY || event.Y >= buttonY+3 {
			continue
		}
		column, sectionW := 2, m.width
		if m.width >= 100 && m.settingsSearch.Value() == "" && action <= 2 {
			side := (m.width - 2) / 2
			column, sectionW = side+4, m.width-side-2
		}
		key := map[int]string{1: "D", 2: "T", 3: "R", 4: "A"}[action]
		label := map[int]string{1: "Verify tools", 2: "Check tracker", 3: "Prepare replacement", 4: "Show advanced"}[action]
		if action == 3 && m.settingsArmed {
			label = "Prepare backup plan"
		}
		if action == 4 && m.settingsAdvanced {
			label = "Hide advanced"
		}
		w := lipgloss.Width(m.settingsButton(key, label, action, max(1, sectionW-8), AppTheme.Accent))
		if event.X >= column && event.X < column+w {
			m.settingsSearch.Blur()
			m.settingsFocus = action
			return m.updateReturningSettings(tea.KeyPressMsg{Code: tea.KeyEnter})
		}
	}
	return m, nil
}

// Choice clicks only move focus. Enter (or its clickable footer hint) remains
// the explicit activation step, including every final consent screen.
func (m model) mouseChoices(event tea.MouseClickMsg, lines []string) (tea.Model, tea.Cmd) {
	var choices []string
	switch m.stage {
	case "confirm":
		choices = []string{"Cancel", "Apply exact reviewed plan"}
	case "returning-remote-confirm":
		choices = []string{"Cancel", "Confirm explicit " + m.returningOperation}
	case "returning-sync":
		choices = []string{"Pull repository (explicit)", "Push existing commits (explicit)"}
	case "source-existing":
		choices = []string{"Continue", "Replace with timestamped backup"}
	case "source-choice":
		choices = []string{"Existing user: clone your repository", "New user: local git init + remote add origin"}
	case "source-recovery":
		choices = []string{"Diagnostics: view contract details & full error", "Retry: re-check contract and repository", "Change source: connect a different repository", "Replace: backup existing checkout and re-initialize"}
	case "source-confirm":
		choices = []string{"Cancel", "Confirm backup and replace"}
	case "tracker-confirm":
		if !m.trackerEditing {
			choices = []string{"Cancel", "Confirm tracker and connection"}
		}
	case "setup":
		choices = []string{"Finish without further adoption", "Track: HOME → repository", "Adopt: repository → HOME"}
	case "preset":
		for _, p := range m.profiles {
			choices = append(choices, safeTerminal(p.Name))
		}
	}
	if m.packageView && m.packageStage == "confirm" {
		choices = []string{"Cancel", "Install exact reviewed missing packages"}
	}
	for i, label := range choices {
		if !mouseLabelHit(lines, event, label) {
			continue
		}
		switch {
		case m.packageView, m.stage == "confirm", m.stage == "returning-remote-confirm", m.stage == "tracker-confirm":
			m.confirmChoice = i
		case strings.HasPrefix(m.stage, "source-"):
			m.sourceChoice = i
		case m.stage == "returning-sync":
			m.syncChoice = i
			m.refreshTabViewport()
		case m.stage == "preset":
			m.presetChoice = i
		case m.stage == "setup":
			m.setupChoice = i
		}
		return m, nil
	}
	if m.packageView && m.packageStage == "" {
		for i, p := range m.packages.Rows {
			kind := p.Manager
			if kind == "brew" {
				kind = p.Kind
			}
			if mouseLabelHit(lines, event, kind+"/"+p.Name) {
				m.packageCursor = i
				break
			}
		}
	}
	if m.stage == "tracker-edit" || m.stage == "tracker-confirm" && m.trackerEditing {
		for i, input := range []string{m.trackerName.View(), m.trackerEmail.View()} {
			if mouseLabelHit(lines, event, strings.TrimSpace(safeTerminal(input))) && m.trackerField != i {
				m.trackerField = i
				m.trackerName.Blur()
				m.trackerEmail.Blur()
				if i == 0 {
					return m, m.trackerName.Focus()
				}
				return m, m.trackerEmail.Focus()
			}
		}
	}
	return m, nil
}

func mouseLabelHit(lines []string, event tea.MouseClickMsg, label string) bool {
	if label == "" || event.Y >= len(lines) {
		return false
	}
	line := lines[event.Y]
	i := strings.Index(line, label)
	if i < 0 {
		return false
	}
	prefix := strings.TrimSpace(strings.TrimLeft(line[:i], "│ "))
	if prefix != "" && prefix != ">" {
		return false
	}
	x := lipgloss.Width(line[:i])
	return event.X >= max(0, x-2) && event.X < x+lipgloss.Width(label)
}

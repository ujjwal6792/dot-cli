package tui

import (
	"strings"

	"charm.land/bubbles/v2/textinput"
	tea "charm.land/bubbletea/v2"
	"charm.land/lipgloss/v2"
)

func ink(text, color string) string {
	return lipgloss.NewStyle().Foreground(lipgloss.Color(color)).Render(text)
}

func (m *model) initSettings() {
	if m.settingsReady {
		return
	}
	m.settingsSearch = textinput.New()
	m.settingsSearch.Prompt = "› "
	m.settingsSearch.Placeholder = "Search sections, tools or paths…"
	m.settingsSearch.CharLimit = 200
	styles := m.settingsSearch.Styles()
	state := textinput.StyleState{
		Text:        lipgloss.NewStyle().Foreground(lipgloss.Color(AppTheme.Title)),
		Prompt:      lipgloss.NewStyle().Foreground(lipgloss.Color(AppTheme.Accent)).Bold(true),
		Placeholder: lipgloss.NewStyle().Foreground(lipgloss.Color(AppTheme.Muted)),
		Suggestion:  lipgloss.NewStyle().Foreground(lipgloss.Color(AppTheme.Muted)),
	}
	styles.Focused, styles.Blurred = state, state
	styles.Cursor.Color = lipgloss.Color(AppTheme.Accent)
	m.settingsSearch.SetStyles(styles)
	m.settingsReady = true
}

func settingField(label, value string, width int) string {
	if value == "" {
		value = "Not available"
	}
	labelW := min(9, max(1, width/3))
	valueW := max(1, width-labelW-1)
	wrapped := lipgloss.NewStyle().Width(valueW).Render(safeTerminal(value))
	lines := strings.Split(wrapped, "\n")
	for i, line := range lines {
		name := strings.Repeat(" ", labelW)
		if i == 0 {
			name = cellLine(label, labelW)
		}
		lines[i] = ink(name, AppTheme.Muted) + " " + lipgloss.NewStyle().Foreground(lipgloss.Color(AppTheme.Title)).Background(lipgloss.Color(AppTheme.Mantle)).Render(cellLine(line, valueW))
	}
	return strings.Join(lines, "\n")
}

func (m model) settingsButton(key, label string, action, width int, color string) string {
	label = cellLine("["+key+"] "+label, min(width, max(1, len(label)+len(key)+3)))
	border, bg := AppTheme.Border, AppTheme.Mantle
	if m.settingsFocus == action {
		border, bg = color, AppTheme.Selection
	}
	if m.loading {
		color = AppTheme.Muted
	}
	edge := func(s string) string { return ink(s, border) }
	text := lipgloss.NewStyle().Foreground(lipgloss.Color(color)).Background(lipgloss.Color(bg)).Bold(m.settingsFocus == action).Render(" " + label + " ")
	line := strings.Repeat("─", lipgloss.Width(label)+2)
	return edge("┌"+line+"┐") + "\n" + edge("│") + text + edge("│") + "\n" + edge("└"+line+"┘")
}

type settingsSection struct {
	id, title, body, border string
	action                  int
}

func (m model) settingsSections(width int) []settingsSection {
	w := max(1, width-4)
	paragraph := func(s string) string {
		return lipgloss.NewStyle().Width(w).Foreground(lipgloss.Color(AppTheme.Muted)).Render(s)
	}
	source, home, state, tracker := configPaths(m.engine)
	if state == "" {
		state = "$XDG_STATE_HOME/dot"
	}
	if tracker == "" {
		tracker = "$XDG_CONFIG_HOME/chezmoi/chezmoi.toml"
	}
	id, osName := m.currentMachine.ID, m.currentMachine.OS
	if id == "" {
		id = "Not loaded"
	}
	if osName == "" {
		osName = "Not loaded"
	}
	machine := settingField("Machine", id, w) + "\n" + settingField("OS", osName, w) + "\n" + paragraph("Local identity · immutable")
	paths := settingField("Source", source, w) + "\n" + settingField("HOME", home, w) + "\n" + settingField("Tracker", tracker, w) + "\n" + settingField("State", state, w)
	tools := []string{}
	for _, name := range []string{"git", "chezmoi"} {
		state, color := "Not checked", AppTheme.Status
		for _, row := range m.dependencies.Rows {
			if row.Tool != name {
				continue
			}
			if row.Problem != "" {
				state, color = row.Problem, AppTheme.Danger
			} else if row.Version != "" {
				state, color = row.Version, AppTheme.Success
			} else if m.dependencies.Ready {
				state, color = "Verified", AppTheme.Success
			}
		}
		tools = append(tools, ink(cellLine(name, 9), AppTheme.Title)+" "+ink(state, color))
	}
	tools = append(tools, m.settingsButton("D", "Verify tools", 1, max(1, w-4), AppTheme.Accent))
	trackerState, trackerColor := "Not checked", AppTheme.Status
	if m.settingsCheckState == "old-user" {
		trackerState, trackerColor = "Contract valid", AppTheme.Success
	} else if m.settingsCheckState == "new-user" {
		trackerState = "Setup required"
	}
	trackerBody := ink(trackerState, trackerColor) + "\n" + paragraph("Local connection · never commits or pushes.") + "\n" + m.settingsButton("T", "Check tracker", 2, max(1, w-4), AppTheme.Accent)
	pending, recovery := recoveryState(m.engine)
	recoveryColor := AppTheme.Success
	if pending {
		recoveryColor = AppTheme.Status
	} else if strings.Contains(recovery, "unavailable") || strings.Contains(recovery, "review") {
		recoveryColor = AppTheme.Muted
	}
	recoveryBody := lipgloss.NewStyle().Width(w).Render(ink(recovery, recoveryColor)) + "\n" + paragraph("Inspect with dot recovery. Rollback requires an exact plan.")
	replaceLabel := "Prepare replacement"
	if m.settingsArmed {
		replaceLabel = "Prepare backup plan"
	}
	sourceBody := paragraph("Backup-first source replacement. The existing checkout is preserved.") + "\n" + m.settingsButton("R", replaceLabel, 3, max(1, w-4), AppTheme.Status)
	advancedBody := m.settingsButton("A", "Show advanced", 4, max(1, w-4), AppTheme.Muted)
	if m.settingsAdvanced {
		advancedBody = m.settingsButton("A", "Hide advanced", 4, max(1, w-4), AppTheme.Accent) + "\n" + ink("Scriptable parity · same engine", AppTheme.Accent) + "\n" + paragraph("dot init --check · status / diff / scope show --format json\nintent plan / preview / apply --approve EXACT_ID\nConsent gates: --ack-unsafe · --ack-desktop · --ack-backup\nPull and Push require explicit confirmation.") + "\n\n" + paragraph("Retained: status, diff, scope, intent, pull, push, contract.\nRetired: legacy runners, automatic commit/push, personal/work profiles and group scopes.\nSetup wizard runs on fresh install only; setup never enrolls files.")
	}
	sections := []settingsSection{
		{"machine", "Machine · read-only", machine, AppTheme.Border, 0},
		{"paths", "Paths · read-only", paths, AppTheme.Accent, 0},
		{"dependencies", "Dependencies", strings.Join(tools, "\n"), AppTheme.Accent, 1},
		{"tracker", "Tracker", trackerBody, AppTheme.Accent, 2},
		{"recovery", "Recovery", recoveryBody, recoveryColor, 0},
		{"source", "Source maintenance", sourceBody, AppTheme.Status, 3},
		{"advanced", "Advanced", advancedBody, AppTheme.Border, 4},
	}
	q := strings.ToLower(strings.TrimSpace(m.settingsSearch.Value()))
	if q == "" {
		return sections
	}
	filtered := []settingsSection{}
	for _, section := range sections {
		if strings.Contains(strings.ToLower(section.title+" "+safeTerminal(section.body)), q) {
			filtered = append(filtered, section)
		}
	}
	return filtered
}

// Sections own their borders; Settings does not wrap them in another frame.
func (m model) settingsLayout() (string, map[int]int) {
	w := max(8, m.width)
	anchors := map[int]int{}
	card := func(s settingsSection, width int) string {
		return borderedPanel(s.title, s.body, width, lipgloss.Height(s.body)+2, s.border)
	}
	if w >= 100 && m.settingsSearch.Value() == "" {
		sideW := (w - 2) / 2
		sections := m.settingsSections(sideW)
		byID := map[string]settingsSection{}
		for _, s := range sections {
			byID[s.id] = s
		}
		left, right := card(byID["machine"], sideW), card(byID["dependencies"], w-sideW-2)
		row1 := lipgloss.JoinHorizontal(lipgloss.Top, left, "  ", right)
		anchors[1] = max(0, lipgloss.Height(right)-4)
		row2Y := lipgloss.Height(row1) + 1
		tracker, recovery := card(byID["tracker"], w-sideW-2), card(byID["recovery"], w-sideW-2)
		row2 := lipgloss.JoinHorizontal(lipgloss.Top, card(byID["paths"], sideW), "  ", tracker+"\n"+recovery)
		anchors[2] = row2Y + max(0, lipgloss.Height(tracker)-4)
		rows := []string{row1, "", row2}
		y := lipgloss.Height(row1) + 1 + lipgloss.Height(row2)
		for _, s := range m.settingsSections(w) {
			if s.id != "source" && s.id != "advanced" {
				continue
			}
			rendered := card(s, w)
			anchors[s.action] = y + 1 + max(0, lipgloss.Height(rendered)-4)
			rows = append(rows, "", rendered)
			y += 1 + lipgloss.Height(rendered)
		}
		return strings.Join(rows, "\n"), anchors
	}
	sections := m.settingsSections(w)
	rows, y := []string{}, 0
	for _, s := range sections {
		if len(rows) > 0 {
			rows = append(rows, "")
			y++
		}
		rendered := card(s, w)
		if s.action > 0 {
			anchors[s.action] = y + max(0, lipgloss.Height(rendered)-4)
		}
		rows = append(rows, rendered)
		y += lipgloss.Height(rendered)
	}
	if len(rows) == 0 {
		return ink("  No matching settings. Esc clears the filter.", AppTheme.Status), anchors
	}
	return strings.Join(rows, "\n"), anchors
}

func (m model) settingsBody() string { body, _ := m.settingsLayout(); return body }

func (m model) renderSettings() string {
	input := m.settingsSearch
	input.SetWidth(max(1, m.width-7))
	color := AppTheme.Border
	if m.settingsFocus == 0 {
		color = AppTheme.Accent
	}
	search := borderedPanel("Find a setting · /", input.View(), m.width, 3, color)
	vp := m.viewport
	offset := vp.YOffset()
	vp.SetWidth(max(1, m.width))
	vp.SetHeight(max(1, m.workspaceHeight()-3))
	vp.SoftWrap = false
	vp.SetContent(m.settingsBody())
	vp.SetYOffset(offset)
	keys := "Tab/Shift+Tab tabs · Ctrl+N/P focus · Enter activate\nD verify tools · T check tracker · R prepare backup · q quit"
	if input.Focused() {
		keys = "Type to filter · Enter finish · Esc clear · Tab next action\nCtrl+U clear text · Ctrl+C quit"
	}
	return m.workspaceHeader() + "\n" + search + "\n" + vp.View() + "\n" + m.renderFooter(keys)
}

func (m *model) focusSetting(action int) tea.Cmd {
	m.settingsFocus = action
	if action == 0 {
		m.settingsSearch.SetWidth(max(1, m.width-7))
		return m.settingsSearch.Focus()
	}
	m.settingsSearch.Blur()
	m.refreshTabViewport()
	_, anchors := m.settingsLayout()
	if y, ok := anchors[action]; ok {
		if y < m.viewport.YOffset() {
			m.viewport.SetYOffset(y)
		} else if y+3 > m.viewport.YOffset()+m.viewport.Height() {
			m.viewport.SetYOffset(y + 3 - m.viewport.Height())
		}
	}
	return nil
}

func (m model) updateReturningSettings(key tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	m.initSettings()
	if m.settingsSearch.Focused() {
		switch key.String() {
		case "esc":
			m.settingsSearch.SetValue("")
			m.settingsSearch.Blur()
			m.settingsFocus = 0
			m.refreshTabViewport()
			m.viewport.GotoTop()
			return m, nil
		case "enter":
			m.settingsSearch.Blur()
			m.settingsFocus = 0
			return m, nil
		case "tab", "shift+tab", "ctrl+n", "ctrl+p": // Move to a visible action below.
		default:
			var cmd tea.Cmd
			m.settingsSearch, cmd = m.settingsSearch.Update(key)
			m.refreshTabViewport()
			m.viewport.GotoTop()
			return m, cmd
		}
	}
	switch key.String() {
	case "esc":
		if m.settingsSearch.Value() != "" {
			m.settingsSearch.SetValue("")
			m.refreshTabViewport()
			m.viewport.GotoTop()
			return m, nil
		}
		return m.performSettingsAction(key)
	case "/":
		cmd := m.focusSetting(0)
		return m, cmd
	case "tab", "shift+tab", "ctrl+n", "ctrl+p":
		ids := []int{0}
		for _, s := range m.settingsSections(m.width) {
			if s.action > 0 {
				ids = append(ids, s.action)
			}
		}
		pos := 0
		for i, id := range ids {
			if id == m.settingsFocus {
				pos = i
			}
		}
		delta := 1
		if key.String() == "shift+tab" || key.String() == "ctrl+p" {
			delta = -1
		}
		cmd := m.focusSetting(ids[(pos+delta+len(ids))%len(ids)])
		return m, cmd
	case "A":
		m.settingsSearch.SetValue("")
		m.settingsAdvanced = !m.settingsAdvanced
		m.refreshTabViewport()
		cmd := m.focusSetting(4)
		return m, cmd
	case "enter":
		if m.settingsFocus == 0 {
			cmd := m.focusSetting(0)
			return m, cmd
		}
		if m.settingsFocus == 4 {
			m.settingsAdvanced = !m.settingsAdvanced
			m.refreshTabViewport()
			cmd := m.focusSetting(4)
			return m, cmd
		}
		shortcut := map[int]string{1: "D", 2: "T", 3: "R"}[m.settingsFocus]
		if shortcut != "" {
			return m.performSettingsAction(tea.KeyPressMsg{Code: rune(shortcut[0]), Text: shortcut})
		}
	case "D", "T", "R":
		m.settingsSearch.SetValue("")
		action := map[string]int{"D": 1, "T": 2, "R": 3}[key.String()]
		m.focusSetting(action)
		return m.performSettingsAction(key)
	case "pgdown":
		m.viewport.ScrollDown(max(1, m.viewport.Height()-1))
	case "pgup":
		m.viewport.ScrollUp(max(1, m.viewport.Height()-1))
	case "home":
		m.viewport.GotoTop()
	case "end":
		m.viewport.GotoBottom()
	default:
		return m.performSettingsAction(key)
	}
	return m, nil
}

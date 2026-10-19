package tui

import (
	"fmt"
	"strings"
	"time"

	tea "charm.land/bubbletea/v2"
	"charm.land/lipgloss/v2"
	"github.com/charmbracelet/x/ansi"
)

// Mouse input expresses the same view actions as keyboard input. It never
// calls policy/effect APIs or turns navigation into Apply consent.
func (m model) mouseKey(action string) (tea.Model, tea.Cmd) {
	key := tea.KeyPressMsg{}
	switch action {
	case "Space", "space":
		key.Code, key.Text = tea.KeySpace, " "
	case "Enter", "enter":
		key.Code = tea.KeyEnter
	case "Esc", "esc":
		key.Code = tea.KeyEsc
	case "Tab", "Tab/Shift+Tab":
		key.Code = tea.KeyTab
	case "Shift+Tab":
		key.Code, key.Mod = tea.KeyTab, tea.ModShift
	case "Ctrl+S", "Ctrl+U", "Ctrl+C", "Ctrl+N/P":
		key.Code, key.Mod = rune(strings.ToLower(action)[5]), tea.ModCtrl
	case "j/k", "J/K", "h/l", "Space/←/→":
		return m.mouseKey(strings.Split(action, "/")[0])
	default:
		if len([]rune(action)) != 1 {
			return m, nil
		}
		key.Code, key.Text = []rune(action)[0], action
	}
	return m.Update(key)
}

func (m model) mouseClick(event tea.MouseClickMsg) (tea.Model, tea.Cmd) {
	if event.X < 0 || event.Y < 0 || event.X >= m.width || event.Y >= m.height || event.Mod&tea.ModShift != 0 {
		return m, nil
	}
	if event.Button == tea.MouseBackward {
		if m.stage == "" || m.hasStatusWorkspace() {
			m.searching = false
			m.search.Blur()
			m.search.SetValue("")
			return m.mouseKey("h")
		}
		return m.mouseKey("Esc")
	}
	if event.Button != tea.MouseLeft {
		m.lastMousePath = ""
		return m, nil
	}
	if m.help {
		return m.mouseKey("Esc")
	}
	if m.returning && !m.packageView {
		// Tab hitboxes use exactly the same complete-label wrapping as tabBar.
		x, y := 0, 1
		for id := TabStatus; id <= TabSettings; id++ {
			w := lipgloss.Width(fmt.Sprintf(" [%d] %s ", id, tabTitle(id)))
			if x > 0 && x+w > m.width {
				x, y = 0, y+1
			}
			if event.Y == y && event.X >= x && event.X < min(m.width, x+w) {
				// Editors own focus until Save/Cancel, just as with keyboard tabs.
				if m.stage == "metadata" || m.stage == "template" || m.stage == "logical" {
					return m, nil
				}
				m.searching = false
				m.search.Blur()
				m.settingsSearch.Blur()
				m.lastMousePath = ""
				return m.switchTab(id)
			}
			x += w
		}
	}
	lines := strings.Split(ansi.Strip(m.View().Content), "\n")
	if action := m.mouseFooterAction(lines, event.X, event.Y); action != "" {
		m.lastMousePath = ""
		if m.searching && action == "/" {
			return m, nil
		}
		if m.searching && action != "/" {
			m.searching = false
			m.search.Blur()
		}
		return m.mouseKey(action)
	}
	if m.loading {
		return m, nil
	}
	if m.stage == "metadata" {
		m.lastMousePath = ""
		return m.mouseMetadata(event, lines)
	}
	if m.stage == "review" && m.selected != nil && m.selected.GatesKnown {
		body := m.renderReview()
		for _, action := range []string{"u", "d", "b"} {
			if i := strings.LastIndex(body, "\n["+action+"] ["); i >= 0 {
				prefix := lipgloss.NewStyle().Width(max(1, m.width-4)).Render(strings.TrimSuffix(body[:i+1], "\n"))
				y := lipgloss.Height(m.workspaceHeader()) + 1 + lipgloss.Height(prefix)
				if event.Y == y && y < lipgloss.Height(m.workspaceHeader())+m.workspaceHeight()-1 && event.X >= 2 && event.X < 11 {
					return m.mouseKey(action)
				}
			}
		}
	}
	if m.stage == "returning-settings" {
		m.lastMousePath = ""
		return m.mouseSettings(event)
	}
	if m.returning && (m.hasStatusWorkspace() || m.stage == "") && !m.packageView {
		return m.mouseBrowser(event)
	}
	m.lastMousePath = ""
	return m.mouseChoices(event, lines)
}

func (m model) mouseFooterAction(lines []string, x, y int) string {
	first := max(0, len(lines)-2)
	if m.initMode || m.packageView {
		first = max(0, len(lines)-5)
	}
	if y < first || y >= len(lines) {
		return ""
	}
	line := lines[y]
	// Only actual hint rows are controls, never the status/error message rows.
	if !strings.Contains(line, " · ") {
		return ""
	}
	column := 0
	for _, part := range strings.Split(line, " · ") {
		w := lipgloss.Width(part)
		if x >= column && x < column+w {
			trimmed := strings.TrimLeft(part, " ")
			key, _, _ := strings.Cut(trimmed, " ")
			allowed := " q r p P a u d b e M m s H D T R A W i L ? / Enter Esc Space Tab/Shift+Tab Ctrl+S Ctrl+U Ctrl+C Ctrl+N/P j/k J/K h/l Space/←/→ "
			if key == "" || !strings.Contains(allowed, " "+key+" ") {
				return ""
			}
			rel := x - column - (w - lipgloss.Width(trimmed))
			if slash := strings.Index(key, "/"); slash > 0 && rel > slash && rel < lipgloss.Width(key) && key != "Space/←/→" {
				return strings.SplitN(key, "/", 2)[1]
			}
			return key
		}
		column += w + 3
	}
	return ""
}

func (m model) mouseBrowser(event tea.MouseClickMsg) (tea.Model, tea.Cmd) {
	status := m.hasStatusWorkspace()
	top, h := lipgloss.Height(m.workspaceHeader()), m.workspaceHeight()
	if event.Y < top || event.Y >= top+h {
		return m, nil
	}
	preview, list, child := m.browserColumns(status)
	if status && m.statusExpanded || event.X < preview {
		m.lastMousePath = ""
		if status && event.Y == top {
			m.searching = false
			m.search.Blur()
			return m.mouseKey("Space")
		}
		return m, nil
	}
	if child > 0 && event.X >= preview+list {
		path := m.browserChildPath(status)
		entries := m.mouseChildren(status, path)
		row := event.Y - top - 1
		start := m.childStart(path, len(entries), h-2)
		if row >= 0 && row < h-2 && start+row < len(entries) {
			entry := entries[start+row]
			m.search.SetValue("")
			m.browserDirs[m.currentTab()] = parentPath(entry.Path)
			if !status {
				m.filter()
			}
			next, cmd := m.mouseFile(entry.Path, entry.Kind, status, false)
			if !status {
				n := next.(model)
				return n, tea.Batch(cmd, n.loadBrowserDirectory(parentPath(entry.Path)))
			}
			return next, cmd
		}
		return m, nil
	}
	if event.Y == top {
		m.lastMousePath = ""
		m.searching = false
		m.search.Blur()
		m.search.SetValue("")
		return m.mouseKey("h")
	}
	if event.Y == top+1 {
		m.lastMousePath = ""
		if !m.searching {
			return m.mouseKey("/")
		}
		return m, nil
	}
	row := event.Y - top - 2
	if row < 0 || row >= m.fileListLimit() {
		return m, nil
	}
	if status {
		files := m.statusFiles()
		start := visibleStart(statusCursor(&m), len(files), m.statusListLimit(), m.listOffsets[TabStatus])
		if start+row < len(files) {
			e := files[start+row]
			return m.mouseFile(e.Path, e.Kind, true, false)
		}
	} else {
		start := visibleStart(m.cursor, len(m.visible), m.fileListLimit(), m.listOffsets[m.currentTab()])
		if start+row < len(m.visible) {
			e := m.entries[m.visible[start+row]]
			checkbox := event.X >= preview+3 && event.X < preview+6
			return m.mouseFile(e.Path, e.Kind, false, checkbox)
		}
	}
	return m, nil
}

func (m model) mouseFile(path, kind string, status, selectFile bool) (tea.Model, tea.Cmd) {
	m.searching = false
	m.search.Blur()
	var focus tea.Cmd
	if status {
		for i, e := range m.statusFiles() {
			if e.Path == path {
				m.tabCursors[TabStatus] = i
				focus = m.statusHighlight()
				break
			}
		}
	} else {
		for row, i := range m.visible {
			if m.entries[i].Path == path {
				m.cursor = row
				focus = m.highlightCurrent()
				break
			}
		}
	}
	now := time.Now()
	double := m.lastMousePath == path && m.lastMouseTab == m.currentTab() && now.Sub(m.lastMouseClick) < 400*time.Millisecond
	m.lastMousePath, m.lastMouseTab, m.lastMouseClick = path, m.currentTab(), now
	if selectFile || double {
		m.lastMousePath = ""
		action := "Space"
		if kind == "directory" && !selectFile {
			action = "l"
		}
		next, cmd := m.mouseKey(action)
		return next, tea.Batch(focus, cmd)
	}
	return m, focus
}

func (m model) mouseChildren(status bool, path string) []Entry {
	var entries []Entry
	if status && m.dashboard != nil {
		for _, e := range dashboardRows(m.dashboard.Managed, path, "", m.hidden) {
			entries = append(entries, Entry{Path: e.Path, Kind: e.Kind})
		}
	} else {
		for _, i := range m.fileRows(path, "") {
			entries = append(entries, m.entries[i])
		}
	}
	return entries
}

func (m model) mouseWheel(event tea.MouseWheelMsg) (tea.Model, tea.Cmd) {
	if event.Mod&tea.ModShift != 0 || event.X < 0 || event.Y < 0 || event.X >= m.width || event.Y >= m.height {
		return m, nil
	}
	delta := 3
	if event.Button == tea.MouseWheelUp {
		delta = -3
	} else if event.Button != tea.MouseWheelDown {
		return m, nil
	}
	m.lastMousePath = ""
	if m.help {
		m.helpOffset = min(15, max(0, m.helpOffset+delta))
		return m, nil
	}
	if m.stage == "metadata" {
		m.editField = min(9, max(0, m.editField+delta/3))
		return m, nil
	}
	if m.returning && (m.hasStatusWorkspace() || m.stage == "") && !m.packageView {
		status := m.hasStatusWorkspace()
		preview, list, child := m.browserColumns(status)
		top := lipgloss.Height(m.workspaceHeader())
		if event.Y < top || event.Y >= top+m.workspaceHeight() {
			return m, nil
		}
		if status && m.statusExpanded || event.X < preview {
			m.resizePreview()
			m.previewViewport.SetYOffset(m.previewViewport.YOffset() + delta)
			return m, nil
		}
		if child > 0 && event.X >= preview+list {
			path := m.browserChildPath(status)
			entries := m.mouseChildren(status, path)
			if m.childOffsets == nil {
				m.childOffsets = map[string]int{}
			}
			m.childOffsets[path] = min(max(0, len(entries)-(m.workspaceHeight()-2)), max(0, m.childStart(path, len(entries), m.workspaceHeight()-2)+delta))
			return m, nil
		}
		if status {
			files := m.statusFiles()
			m.tabCursors[TabStatus] = min(max(0, len(files)-1), max(0, statusCursor(&m)+delta))
			m.listOffsets[TabStatus] = max(0, m.listOffsets[TabStatus]+delta)
			return m, m.statusHighlight()
		}
		m.cursor = min(max(0, len(m.visible)-1), max(0, m.cursor+delta))
		m.listOffsets[m.currentTab()] = max(0, m.listOffsets[m.currentTab()]+delta)
		return m, m.highlightCurrent()
	}
	if m.stage == "template" || m.stage == "logical" || m.packageView && (m.packageStage == "edit" || m.packageStage == "import") {
		action := "down"
		if delta < 0 {
			action = "up"
		}
		key := tea.KeyPressMsg{Code: tea.KeyDown}
		if action == "up" {
			key.Code = tea.KeyUp
		}
		var cmd tea.Cmd
		m.templateInput, cmd = m.templateInput.Update(key)
		return m, cmd
	}
	if m.packageView && m.packageStage == "" {
		m.packageCursor = min(max(0, len(m.packages.Rows)-1), max(0, m.packageCursor+delta))
		return m, nil
	}
	m.viewport.SetYOffset(m.viewport.YOffset() + delta)
	return m, nil
}

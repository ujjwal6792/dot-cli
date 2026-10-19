package tui

import (
	"fmt"
	"strings"

	"charm.land/lipgloss/v2"
)

func (m model) browserPreviewWidth() int {
	if m.returning && m.currentTab() == TabStatus && m.statusExpanded {
		return m.width
	}
	if m.width < 60 {
		return 0
	}
	return min(38, max(20, m.width/4))
}

func (m model) browserInfoHeight() int { return min(5, max(2, m.workspaceHeight()/3)) }

// One selection, one inline preview. Whole-repository inspection remains
// available through the CLI instead of taking over the browsing workspace.
func (m model) previewText() string {
	if !m.returning {
		return m.renderHighlight()
	}
	if m.highlight.Status == "directory" {
		text := "l / Enter opens this folder."
		if m.currentTab() != TabStatus {
			if m.trackedBrowser() || m.mode != "track" {
				text += "\nSpace selects exact tracked files."
			} else {
				text += "\nSpace opens tracking options."
			}
		}
		return text
	}
	if m.currentTab() == TabTrack {
		if i := m.current(); i >= 0 && m.untrackedEntry(m.entries[i]) && m.entries[i].Status == "eligible" {
			return "Untracked\nSpace opens tracking options."
		}
	}
	if len(m.highlight.Diffs) == 0 && m.highlight.Text == "" {
		if m.highlight.Status == "managed" || m.highlight.Status == "eligible" {
			return "No changes."
		}
		return safeTerminal(m.highlight.Summary)
	}
	// The inspector already names the file and its status; retain the styled
	// diff without the repeated direction/authorization banner.
	lines := strings.Split(m.renderHighlight(), "\n")
	return strings.Join(lines[min(2, len(lines)):], "\n")
}

func (m model) browserInfo(status bool) string {
	if status {
		files := m.statusFiles()
		if c := statusCursor(&m); c >= 0 {
			e := files[c]
			if e.Kind == "directory" {
				return safeTerminal(e.Path) + "/\n" + folderSummary(e.Tracked, e.Changed, e.Missing)
			}
			return safeTerminal(e.Path) + "\n" + dashboardState(e.State) + "\n" + safeTerminal(e.Portability) + "\n" + safeTerminal(e.Impact) + "\n" + safeTerminal(e.Assignment)
		}
		return "Select a tracked file\nto inspect its diff."
	}
	if i := m.current(); i >= 0 {
		e := m.entries[i]
		if e.Kind == "directory" {
			return safeTerminal(e.Path) + "/\n" + m.fileFolderSummary(e.Path)
		}
		return safeTerminal(e.Path) + "\n" + safeTerminal(e.Status) + "\nScope: " + fmt.Sprint(e.Approved) + "\n" + safeTerminal(inputValue(e.Metadata["portability"])) + "\n" + assignmentDescription(e.Metadata)
	}
	return "Select a file\nto inspect its diff."
}

func browserRow(name, state string, width int, active bool, prefix string, untracked, changedFolder bool) string {
	stateW := min(lipgloss.Width(state), max(0, width/2-2))
	nameW := max(1, width-lipgloss.Width(prefix)-stateW-2)
	style := lipgloss.NewStyle().Foreground(lipgloss.Color(AppTheme.Text))
	if active {
		style = style.Foreground(lipgloss.Color(AppTheme.Title)).Background(lipgloss.Color(AppTheme.Selection)).Bold(true)
	}
	if untracked {
		style = style.Foreground(lipgloss.Color("#929292"))
	}
	nameStyle := style
	if changedFolder {
		nameStyle = nameStyle.Foreground(lipgloss.Color(AppTheme.Status))
	}
	return cellLine(style.Render(prefix)+nameStyle.Render(cellLine(name, nameW))+style.Render("  "+cellLine(state, stateW)), width)
}

func (m model) statusBrowserList(width, height int) string {
	files := m.statusFiles()
	cur := statusCursor(&m)
	rows := []string{m.filterLine()}
	start := visibleStart(cur, len(files), max(1, height-2), m.listOffsets[TabStatus])
	for i := start; i < min(len(files), start+max(1, height-2)); i++ {
		e := files[i]
		name := browserName(e.Path, m.search.Value())
		state := dashboardState(e.State)
		if e.Kind == "directory" {
			name += "/"
			state = folderSummary(e.Tracked, e.Changed, e.Missing)
		}
		prefix := "  "
		if i == cur {
			prefix = "> "
		}
		rows = append(rows, browserRow(name, state, width, i == cur, prefix, false, e.Kind == "directory" && e.Changed+e.Missing > 0))
	}
	if len(files) == 0 {
		empty := "Nothing tracked yet."
		if m.search.Value() != "" {
			empty = "No matches · Esc clears search."
		}
		if m.dashboardLoading {
			empty = "Loading status…"
		}
		rows = append(rows, empty)
	}
	rows = append(rows, fmt.Sprintf("%d / %d items", max(0, cur+1), len(files)))
	return region(strings.Join(rows, "\n"), width, height)
}

func (m model) fileBrowserList(width, height int) string {
	rows := []string{m.filterLine()}
	start := visibleStart(m.cursor, len(m.visible), max(1, height-2), m.listOffsets[m.currentTab()])
	for row := start; row < min(len(m.visible), start+max(1, height-2)); row++ {
		e := m.entries[m.visible[row]]
		name := browserName(e.Path, m.search.Value())
		mark := "[ ] "
		if _, ok := m.basket[e.Path]; ok {
			mark = "[x] "
		}
		state := e.Status
		untracked := m.untrackedEntry(e)
		changedFolder := false
		if e.Kind == "directory" {
			tracked, changed, missing := m.fileFolderStats(e.Path)
			name += "/"
			changedFolder = changed+missing > 0
			state = folderSummary(tracked, changed, missing)
			for p := range m.basket {
				if strings.HasPrefix(p, e.Path+"/") {
					mark = "[-] "
					break
				}
			}
		} else if e.TargetStatus != "" {
			state = dashboardState(e.TargetStatus)
		}
		if untracked && (e.Kind == "directory" || e.Status == "eligible") {
			state = "untracked"
		}
		cursor := " "
		if row == m.cursor {
			cursor = ">"
		}
		rows = append(rows, browserRow(name, state, width, row == m.cursor, cursor+mark, untracked, changedFolder))
	}
	if len(m.visible) == 0 {
		reason, guidance := m.emptyReason()
		rows = append(rows, reason, guidance)
	}
	rows = append(rows, fmt.Sprintf("%d / %d items · %d selected", min(m.cursor+1, len(m.visible)), len(m.visible), len(m.basket)))
	return region(strings.Join(rows, "\n"), width, height)
}

func (m model) browserChildren(status bool, width, height int) (string, string) {
	var path string
	var rows []string
	if status {
		files := m.statusFiles()
		if c := statusCursor(&m); c >= 0 && files[c].Kind == "directory" {
			path = files[c].Path
			children := dashboardRows(m.dashboard.Managed, path, "", m.hidden)
			start := m.childStart(path, len(children), height)
			for _, e := range children[start:] {
				name, detail := safeTerminal(pathBase(e.Path)), dashboardState(e.State)
				if e.Kind == "directory" {
					name += "/"
					detail = folderSummary(e.Tracked, e.Changed, e.Missing)
				}
				rows = append(rows, browserRow(name, detail, width, false, "", false, e.Kind == "directory" && e.Changed+e.Missing > 0))
				if len(rows) >= height {
					break
				}
			}
		}
	} else if i := m.current(); i >= 0 && m.entries[i].Kind == "directory" {
		path = m.entries[i].Path
		children := m.fileRows(path, "")
		start := m.childStart(path, len(children), height)
		for _, index := range children[start:] {
			e := m.entries[index]
			name, detail := safeTerminal(pathBase(e.Path)), e.Status
			changedFolder := false
			if e.Kind == "directory" {
				tracked, changed, missing := m.fileFolderStats(e.Path)
				name += "/"
				changedFolder = changed+missing > 0
				detail = folderSummary(tracked, changed, missing)
			}
			untracked := m.untrackedEntry(e)
			if untracked && (e.Kind == "directory" || e.Status == "eligible") {
				detail = "untracked"
			}
			rows = append(rows, browserRow(name, detail, width, false, "", untracked, changedFolder))
			if len(rows) >= height {
				break
			}
		}
		if len(rows) == 0 {
			rows = append(rows, "l / Enter loads contents.")
		}
	}
	return path, strings.Join(rows, "\n")
}

func (m model) browserChildPath(status bool) string {
	if status {
		files := m.statusFiles()
		if c := statusCursor(&m); c >= 0 && files[c].Kind == "directory" {
			return files[c].Path
		}
	} else if i := m.current(); i >= 0 && m.entries[i].Kind == "directory" {
		return m.entries[i].Path
	}
	return ""
}

func (m model) browserColumns(status bool) (preview, list, child int) {
	preview = m.browserPreviewWidth()
	list = m.width - preview
	if list >= 86 && m.search.Value() == "" && m.browserChildPath(status) != "" {
		child = min(40, list/3)
		list -= child
	}
	return
}

func (m model) childStart(path string, count, height int) int {
	return min(max(0, m.childOffsets[path]), max(0, count-max(1, height)))
}

func (m model) renderBrowserWorkspace(status bool) string {
	h := m.workspaceHeight()
	previewW, listW, childW := m.browserColumns(status)
	if status && m.statusExpanded {
		infoH := m.browserInfoHeight()
		info := region(m.browserInfo(true), m.width-4, infoH)
		body := info + "\n" + m.previewRegion(m.width-4, h-3-infoH)
		return m.workspaceFrame(panel("Status & diff · expanded", body, m.width, h, true), true)
	}
	dir := m.browserDirs[m.currentTab()]
	title := "Tracked Files · " + browserPath(dir)
	if !status {
		title = strings.ToUpper(m.mode) + " · " + browserPath(dir)
		if m.mode == "track" {
			if m.trackedBrowser() {
				title += " · tracked"
			} else {
				title += " · all HOME"
			}
		}
	}
	if m.search.Value() != "" {
		title = "Search results · " + m.search.Value()
	}
	path, children := m.browserChildren(status, childW-4, h-2)
	body := m.fileBrowserList(listW-4, h-2)
	if status {
		body = m.statusBrowserList(listW-4, h-2)
	}
	browser := panel(title, body, listW, h, true)
	if childW > 0 {
		browser = lipgloss.JoinHorizontal(lipgloss.Top, browser, panel(safeTerminal(pathBase(path))+"/", children, childW, h, false))
	}
	if previewW > 0 {
		infoH := m.browserInfoHeight()
		info := region(m.browserInfo(status), previewW-4, infoH)
		preview := info + "\n" + m.previewRegion(previewW-4, h-3-infoH)
		browser = lipgloss.JoinHorizontal(lipgloss.Top, panel("Status & diff", preview, previewW, h, false), browser)
	}
	return m.workspaceFrame(browser, status)
}

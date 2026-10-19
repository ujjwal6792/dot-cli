package tui

import (
	"fmt"
	"strings"

	"charm.land/lipgloss/v2"
	"github.com/charmbracelet/x/ansi"
)

// All workspace dimensions are terminal cells, including borders. A row is
// truncated before styling, so long paths cannot change the list's geometry.
func cellLine(s string, width int) string {
	width = max(0, width)
	s = strings.ReplaceAll(s, "\t", "  ")
	s = ansi.Truncate(strings.ReplaceAll(s, "\n", " "), width, "…")
	return s + strings.Repeat(" ", max(0, width-lipgloss.Width(s)))
}

func region(s string, width, height int) string {
	lines := strings.Split(s, "\n")
	out := make([]string, max(0, height))
	for i := range out {
		line := ""
		if i < len(lines) {
			line = lines[i]
		}
		out[i] = cellLine(line, width)
	}
	return strings.Join(out, "\n")
}

func surface(s string, width, height int, bg string) string {
	return lipgloss.NewStyle().Foreground(lipgloss.Color(AppTheme.Text)).Background(lipgloss.Color(bg)).Render(region(s, width, height))
}

func panel(title, body string, width, height int, focused bool) string {
	border := AppTheme.Border
	if focused {
		border = AppTheme.Accent
	}
	return borderedPanel(title, body, width, height, border)
}

func borderedPanel(title, body string, width, height int, border string) string {
	if width < 4 || height < 3 {
		return surface(body, width, height, AppTheme.Bg)
	}
	edge := lipgloss.NewStyle().Foreground(lipgloss.Color(border)).Background(lipgloss.Color(AppTheme.Bg))
	title = ansi.Truncate(title, max(0, width-6), "…")
	titleColor := border
	if border == AppTheme.Border {
		titleColor = AppTheme.Title
	}
	head := lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color(titleColor)).Background(lipgloss.Color(AppTheme.Bg)).Render(title)
	rows := []string{edge.Render("┌─ ") + head + edge.Render(" "+strings.Repeat("─", max(0, width-6-lipgloss.Width(title)))+"─┐")}
	for _, line := range strings.Split(region(body, width-4, height-2), "\n") {
		rows = append(rows, edge.Render("│")+surface(" "+line+" ", width-2, 1, AppTheme.Bg)+edge.Render("│"))
	}
	rows = append(rows, edge.Render("└"+strings.Repeat("─", width-2)+"┘"))
	return strings.Join(rows, "\n")
}

func (m model) workspaceHeader() string {
	identity := "machine: —"
	if m.currentMachine.ID != "" {
		identity = fmt.Sprintf("machine: %s [%s]", safeTerminal(m.currentMachine.ID), safeTerminal(m.currentMachine.OS))
	}
	brand := "● dot · Dotfiles"
	if m.width >= 90 {
		brand += "  |  Packages (P)"
	}
	line := ink(brand, AppTheme.Accent) + "   " + ink(identity, AppTheme.Title)
	if m.width >= 115 {
		line += "   git: system"
	}
	return surface(line, m.width, 1, AppTheme.Mantle) + "\n" + m.tabBar()
}

func (m model) hasStatusWorkspace() bool {
	_, supported := m.engine.(StatusEngine)
	return m.stage == "returning-home" && m.activeTab == TabStatus && (supported || m.dashboard != nil)
}

func (m model) workspaceMessage() string {
	if m.err != nil {
		return "Error: " + safeTerminal(m.err.Error())
	}
	if m.dashboardErr != nil && m.currentTab() == TabStatus {
		return "Status unavailable: " + safeTerminal(m.dashboardErr.Error())
	}
	if m.loading || m.dashboardLoading {
		return "Loading…  Navigation is available; no files are changed."
	}
	if m.message != "" {
		return safeTerminal(m.message)
	}
	if m.stage == "returning-settings" {
		return "Settings · Local connection and tools · Explicit review protects changes."
	}
	if m.currentTab() == TabStatus && m.returning {
		return "Read-only inspection · No files preselected; inspection is not consent."
	}
	return fmt.Sprintf("%d selected · %s · Nothing is applied until the exact plan is confirmed.", len(m.basket), m.mode)
}

func (m model) workspaceFooter(status bool) string {
	keys := "Space expand preview · Tab/Shift+Tab tabs · j/k move · h/l folders\n/ search tracked · J/K preview · p refresh · H hidden · r rescan · q quit · ? help"
	if status && m.statusExpanded {
		keys = "Space / Esc collapse preview · J/K scroll · j/k files\nTab/Shift+Tab tabs · p refresh · q quit · ? help"
	}
	if !status {
		filter := ""
		if m.returning && m.mode == "track" {
			filter = "f tracked only · "
			if m.trackedBrowser() {
				filter = "f all HOME · "
			}
		}
		keys = "q quit · Tab/Shift+Tab tabs · j/k move\nSpace track/select · " + filter + "h/l folders · p preview · ? help"
		if m.width >= 110 {
			keys = "Tab/Shift+Tab tabs · 1-6 jump · j/k move · h/l folders · Space track/select · p preview · q quit\n? help · / search · " + filter + "e edit · M logical · m machines · s profiles · H hidden · r rescan"
		}
	}
	switch m.stage {
	case "done":
		keys = "Enter return to Status · q quit"
	case "review":
		keys = "j/k scroll · p refresh plan · a continue · Esc cancel\nu unsafe · d desktop · b backup · q cancel · ? help"
	case "confirm":
		keys = "j/k choose · Enter confirm · Esc cancel\nCancel is selected by default · ? help"
	case "metadata":
		keys = "Tab/Shift+Tab field · Enter save · Esc cancel\nType edit · Space/←/→ choices · Ctrl+U clear"
	case "template", "logical":
		keys = "Ctrl+S save · Esc cancel\nChanges require an exact reviewed plan before applying"
	case "preset":
		keys = "j/k move · Enter select · Esc cancel\nProfile selection does not apply files"
	}
	return m.renderFooter(keys)
}

func (m model) workspaceHeight() int {
	return max(1, m.height-lipgloss.Height(m.workspaceHeader())-6)
}

func (m model) workspaceFrame(body string, status bool) string {
	return m.workspaceHeader() + "\n" + region(body, m.width, m.workspaceHeight()) + "\n" + m.workspaceFooter(status)
}

func (m model) sidebarWidth() int { return min(38, max(26, m.width/3)) }

func (m model) filterLine() string {
	if m.searching {
		return m.search.View()
	}
	if m.search.Value() != "" {
		return "/ " + safeTerminal(m.search.Value()) + "  (Esc clear)"
	}
	return "/ filter (press /)…"
}

func (m model) machineContext(width int) string {
	identity := "Machine: unknown · immutable"
	if m.currentMachine.ID != "" {
		identity = fmt.Sprintf("Machine: %s [%s]", safeTerminal(m.currentMachine.ID), safeTerminal(m.currentMachine.OS))
	}
	source, _, state, tracker := configPaths(m.engine)
	rows := []string{identity}
	if source != "" {
		rows = append(rows, "Repo: "+safeTerminal(source))
	}
	if tracker != "" {
		rows = append(rows, "Config: "+safeTerminal(tracker))
	}
	if state != "" {
		rows = append(rows, "State: "+safeTerminal(state))
	}
	return panel("Tracker Connection", strings.Join(rows, "\n"), width, len(rows)+2, false)
}

func listRow(text string, width int, active bool) string {
	style := lipgloss.NewStyle().Foreground(lipgloss.Color(AppTheme.Text))
	if active {
		style = style.Foreground(lipgloss.Color(AppTheme.Title)).Background(lipgloss.Color(AppTheme.Selection)).Bold(true)
	}
	return style.Render(cellLine(text, width))
}

func visibleStart(cursor, count, limit, previous int) int {
	limit = max(1, limit)
	start := min(max(0, previous), max(0, count-limit))
	if cursor < start {
		start = max(0, cursor)
	}
	if cursor >= start+limit {
		start = cursor - limit + 1
	}
	return start
}

func (m model) fileListLimit() int {
	h := m.workspaceHeight()
	if m.returning {
		return max(1, h-4)
	}
	if m.width < 72 {
		return max(1, max(5, h/2)-4)
	}
	contextH := min(lipgloss.Height(m.machineContext(m.sidebarWidth()-4)), max(0, h-9))
	return max(1, h-4-contextH)
}

func (m model) statusListLimit() int { return max(1, m.workspaceHeight()-4) }

func (m *model) keepFileCursorVisible() {
	id := m.currentTab()
	m.listOffsets[id] = visibleStart(m.cursor, len(m.visible), m.fileListLimit(), m.listOffsets[id])
}

func (m model) fileList(width, height int) string {
	var rows []string
	selectedFolders := map[string]bool{}
	for path := range m.basket {
		for parent := parentPath(path); parent != ""; parent = parentPath(parent) {
			selectedFolders[parent] = true
		}
	}
	rows = append(rows, m.filterLine())
	if len(m.visible) == 0 {
		reason, guidance := m.emptyReason()
		if m.loading {
			reason, guidance = "Loading files…", ""
		}
		rows = append(rows, reason, guidance)
	} else {
		limit := max(1, height-2)
		start := visibleStart(m.cursor, len(m.visible), limit, m.listOffsets[m.currentTab()])
		for row := start; row < min(len(m.visible), start+limit); row++ {
			e := m.entries[m.visible[row]]
			mark, cursor, folder := "[ ]", " ", ""
			if _, ok := m.basket[e.Path]; ok {
				mark = "[x]"
			}
			if e.Kind == "directory" {
				if selectedFolders[e.Path] {
					mark = "[-]"
				}
				if e.Open {
					folder = "/ ▾"
				} else {
					folder = "/ ▸"
				}
			}
			if row == m.cursor {
				cursor = ">"
			}
			rows = append(rows, listRow(cursor+mark+" "+safeTerminal(e.Path)+folder, width, row == m.cursor))
		}
		rows = append(rows, fmt.Sprintf("%d / %d · %d selected", m.cursor+1, len(m.visible), len(m.basket)))
	}
	return region(strings.Join(rows, "\n"), width, height)
}

func (m model) previewRegion(width, height int) string {
	if m.previewLoading {
		return region("Loading preview…", width, height)
	}
	if m.highlight.Status == "" {
		if m.inventoryErr != nil {
			return region("Couldn't list files\nFix the error, then press r to rescan.", width, height)
		}
		return region("Select a file to inspect its diff.\nJ/K scroll · p reviews the exact plan.", width, height)
	}
	vp := m.previewViewport
	offset := vp.YOffset()
	vp.SetWidth(max(1, width))
	vp.SetHeight(max(1, height))
	vp.SoftWrap = true
	vp.SetContent(m.previewText())
	vp.SetYOffset(offset)
	return region(vp.View(), width, height)
}

func (m model) fileDetail() string {
	direction := "HOME → REPOSITORY"
	if m.mode == "adopt" {
		direction = "REPOSITORY → TARGET"
	}
	if m.mode == "untrack" {
		direction = "REPOSITORY → STOP TRACKING · live files unchanged"
	}
	lines := []string{ink(direction, AppTheme.Accent), ink(fmt.Sprintf("%d selected · [p] Plan Changes", len(m.basket)), AppTheme.Success)}
	if i := m.current(); i >= 0 {
		e := m.entries[i]
		lines = append(lines, safeTerminal(e.Path), fmt.Sprintf("Status: %s · Scope: %t", safeTerminal(e.Status), e.Approved))
		meta := []string{}
		for _, k := range []string{"portability", "impact"} {
			if v := inputValue(e.Metadata[k]); v != "" {
				meta = append(meta, v)
			}
		}
		if e.TargetStatus != "" {
			meta = append(meta, "target: "+e.TargetStatus)
		}
		if len(meta) > 0 {
			lines = append(lines, strings.Join(meta, " · "))
		}
		lines = append(lines, "Assignment: "+m.assignmentTarget())
	} else {
		reason, extra := m.emptyReason()
		lines = append(lines, reason, extra)
	}
	return strings.Join(lines, "\n")
}

func (m model) assignmentTarget() string {
	if i := m.current(); i >= 0 {
		return assignmentDescription(m.entries[i].Metadata)
	}
	return "All machines"
}

func assignmentDescription(metadata map[string]any) string {
	label := "All machines"
	if v, ok := metadata["machines"].(map[string]any); ok {
		include, exclude := metadataList(v["include"]), metadataList(v["exclude"])
		if len(include) > 0 && !has(include, "all") {
			label = strings.Join(include, ", ") + " only"
		}
		if len(exclude) > 0 {
			label += " except " + strings.Join(exclude, ", ")
		}
	}
	return safeTerminal(label)
}

func (m model) renderLegacyFileWorkspace() string {
	h := m.workspaceHeight()
	if m.width < 72 {
		listH := max(5, h/2)
		mainH := max(1, h-listH)
		body := panel(strings.ToUpper(m.mode)+" · Files", m.fileList(m.width-4, listH-2), m.width, listH, true)
		body += "\n" + panel("PREVIEW", m.previewRegion(m.width-4, mainH-2), m.width, mainH, false)
		return m.workspaceFrame(body, false)
	}
	sideW := m.sidebarWidth()
	context := m.machineContext(sideW - 4)
	contextH := min(lipgloss.Height(context), max(0, h-9))
	listH := h - 2 - contextH
	left := m.fileList(sideW-4, listH)
	if contextH > 0 {
		left += "\n" + region(context, sideW-4, contextH)
	}
	left = panel("Files · "+strings.ToUpper(m.mode), left, sideW, h, true)
	mainW := m.width - sideW
	detail := m.fileDetail()
	detailH := min(6, max(2, h-6))
	diffH := max(1, h-2-detailH)
	diff := panel("Diff preview · J/K", m.previewRegion(mainW-8, diffH-2), mainW-4, diffH, false)
	body := region(detail, mainW-4, detailH) + "\n" + diff
	right := panel(tabTitle(m.currentTab()), body, mainW, h, false)
	return m.workspaceFrame(lipgloss.JoinHorizontal(lipgloss.Top, left, right), false)
}

func (m model) renderStatusWorkspace() string { return m.renderBrowserWorkspace(true) }

func (m model) renderFileWorkspace() string {
	if m.returning {
		return m.renderBrowserWorkspace(false)
	}
	return m.renderLegacyFileWorkspace()
}

func (m *model) resizePreview() {
	if m.returning {
		offset := m.previewViewport.YOffset()
		m.previewViewport.SetWidth(max(1, m.browserPreviewWidth()-4))
		m.previewViewport.SetHeight(max(1, m.workspaceHeight()-2-m.browserInfoHeight()-1))
		m.previewViewport.SoftWrap = true
		m.previewViewport.SetContent(m.previewText())
		m.previewViewport.SetYOffset(offset)
		return
	}
	h := m.workspaceHeight()
	w := m.width - m.sidebarWidth() - 8
	previewH := h - min(6, max(2, h-6)) - 4
	if m.returning && m.currentTab() == TabStatus {
		previewH = h - min(3, max(0, h-5)) - 4
	}
	if m.width < 72 {
		w = m.width - 4
		previewH = h - max(5, h/2) - 2
		if m.returning && m.currentTab() == TabStatus {
			mainH := h - max(4, h/2)
			previewH = mainH - min(3, max(0, mainH-5)) - 3
		}
	}
	offset := m.previewViewport.YOffset()
	m.previewViewport.SetWidth(max(1, w))
	m.previewViewport.SetHeight(max(1, previewH))
	m.previewViewport.SoftWrap = true
	if m.highlight.Status != "" {
		m.previewViewport.SetContent(m.renderHighlight())
	}
	m.previewViewport.SetYOffset(offset)
}

func (m model) renderTextWorkspace(title, body, keys string) string {
	h := m.workspaceHeight()
	vp := m.viewport
	offset := vp.YOffset()
	vp.SetWidth(max(1, m.width-4))
	vp.SetHeight(max(1, h-2))
	vp.SoftWrap = true
	vp.SetContent(body)
	vp.SetYOffset(offset)
	footer := m.renderFooter(keys + "\n1-6 tabs · J/K scroll · W settings · q quit=Finish without changes")
	return m.workspaceHeader() + "\n" + panel(title, vp.View(), m.width, h, true) + "\n" + footer
}

func (m *model) resizeTextViewport() {
	if m.stage != "returning-sync" && m.stage != "returning-settings" && m.stage != "returning-inspect" {
		return
	}
	offset := m.viewport.YOffset()
	m.viewport.SetWidth(max(1, m.width-4))
	m.viewport.SetHeight(max(1, m.workspaceHeight()-2))
	m.viewport.SoftWrap = true
	if m.stage == "returning-settings" {
		m.viewport.SetWidth(max(1, m.width))
		m.viewport.SetHeight(max(1, m.workspaceHeight()-3))
		m.viewport.SoftWrap = false
	}
	m.viewport.SetYOffset(offset)
}

func (m model) renderKeyHelp() string {
	lines := []string{
		"Navigation", "Tab/Shift+Tab Next / previous tab (wraps 1–6)", "1–6          Jump to a tab", "j/k, ↑/↓     Move through files", "PgUp/PgDn    Move one page", "Home/End     First / last file", "h/←          Collapse folder / move to parent", "l/→, Enter   Open folder", "f            Track: all HOME files / tracked only", "/            Search paths; all HOME Enter searches unloaded folders", "J/K          Scroll diff", "",
		"Selection and review", "Space        Status: expand/collapse preview; Track: options/select", "e            Edit metadata", "M            Edit logical file metadata", "m            Retarget Track assignment", "Ctrl+N/P     Focus next / previous Settings control", "s            Compose Adopt profiles", "p            Review exact plan (Status: read-only diff)", "a            Continue to Cancel-default confirmation", "u/d/b        Separate required review acknowledgments", "",
		"Other actions", "H            Toggle hidden files", "r            Rescan inventory", "P            Packages (Git Sync: explicit push)", "W            Settings", "Esc          Back / clear filter / cancel", "q            Quit", "", "Browsing never authorizes changes. Apply requires an exact reviewed plan.",
	}
	offset := min(m.helpOffset, max(0, len(lines)-(m.workspaceHeight()-2)))
	body := panel("Keyboard help · j/k scroll · Esc close", strings.Join(lines[offset:], "\n"), m.width, m.workspaceHeight(), true)
	return m.workspaceFrame(body, m.returning && m.currentTab() == TabStatus)
}

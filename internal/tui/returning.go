package tui

import (
	"context"
	"fmt"
	"strings"

	"charm.land/bubbles/v2/table"
	tea "charm.land/bubbletea/v2"
	"charm.land/lipgloss/v2"
)

type ReturningEngine interface {
	Inspect(context.Context, string) (string, error)
	Remote(context.Context, string, bool) error
}

// DashboardEntry is one tracked file for the returning-home status table.
type DashboardEntry struct {
	Path        string
	Dest        string
	State       string
	Portability string
	Impact      string
	Assignment  string
	Kind        string
	Tracked     int
	Changed     int
	Missing     int
}

// DashboardData feeds the always-visible status table on returning-home.
// Managed lists tracked files; Adoptable counts repository files not yet
// applied here; Diffs counts files whose target differs (-1 when unknown).
type DashboardData struct {
	Machine   Machine
	Managed   []DashboardEntry
	Adoptable int
	Diffs     int
}

// StatusEngine is optional: engines that can summarize tracked state feed
// the dashboard. Engines without it show the menu alone, as before.
type StatusEngine interface {
	DashboardStatus(context.Context) (DashboardData, error)
}
type returningMsg struct {
	generation uint64
	text       string
	err        error
	remote     bool
}
type dashboardMsg struct {
	generation uint64
	data       *DashboardData
	err        error
}

// dashboardCommand loads the returning-home status table. Nil unless the
// engine implements StatusEngine, so fixture engines behave exactly as
// before. Reads are read-only; issuing one authorizes nothing.
func (m model) dashboardCommand() tea.Cmd {
	e, ok := m.engine.(StatusEngine)
	if !ok || !m.returning {
		return nil
	}
	return func() tea.Msg {
		data, err := e.DashboardStatus(m.setupCtx())
		return dashboardMsg{m.tabGeneration, &data, err}
	}
}

var returningActions = []string{"Finish without changes", "Track files: HOME → repository", "Adopt files: repository → HOME", "Status (machine-scoped)", "Diff: preview importable changes", "Scope: approved files", "Pull repository (explicit)", "Push existing commits (explicit)"}

// returningDescriptions explains the consequence of the highlighted action.
// Same order as returningActions; read-only items say so.
var returningDescriptions = []string{
	"Leave everything unchanged and quit.",
	"Version new HOME files into the repository.",
	"Apply repository files onto this machine.",
	"Read-only view of tracked state.",
	"Read-only preview of changes you could import.",
	"Read-only list of already-approved files.",
	"Fast-forward pull only; adoption never happens here.",
	"Push existing commits; nothing is committed for you.",
}

func NewReturningModel(e Engine) tea.Model {
	return NewReturningModelContext(context.Background(), e)
}

func NewReturningModelContext(ctx context.Context, e Engine) tea.Model {
	m := NewModel(e, false, "track").(model)
	m.setupContext, m.setupCancel = context.WithCancel(ctx)
	m.returning = true
	m.activeTab = TabStatus
	m.stage = "returning-home"
	m.message = ""
	_, m.dashboardLoading = e.(StatusEngine)
	return m
}
func (m *model) resetReturning() {
	m.stage = "returning-home"
	m.activeTab = TabStatus
	m.returningChoice = 0
	m.confirmChoice = 0
	m.syncChoice = 0
	m.settingsArmed = false
	m.basket = map[string]map[string]any{}
	m.metadataEdits = map[string]map[string]any{}
	m.selected = nil
	m.selectedProfiles = nil
	m.logicalRequest = nil
	m.adoptTouched = false
	m.planFresh = false
	m.unsafeAck = false
	m.desktopAck = false
	m.backupAck = false
	m.loading = false
	m.err = nil
	m.generation++
	m.previewGeneration++
	m.inventoryGeneration++
	m.message = ""
}
func (m model) updateReturning(key tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	if m.stage == "returning-settings" && m.settingsSearch.Focused() {
		return m.updateReturningSettings(key)
	}
	if key.String() == "ctrl+c" || key.String() == "q" {
		if m.setupCancel != nil {
			m.setupCancel()
		}
		if key.String() == "ctrl+c" {
			return m, tea.Interrupt
		}
		return m, tea.Quit
	}
	if m.loading {
		return m, nil
	}
	switch key.String() {
	case "1":
		return m.switchTab(TabStatus)
	case "2":
		return m.switchTab(TabTrack)
	case "3":
		return m.switchTab(TabAdopt)
	case "4":
		return m.switchTab(TabUntrack)
	case "5":
		return m.switchTab(TabSync)
	case "6", "W":
		return m.switchTab(TabSettings)
	}
	if m.stage == "returning-settings" {
		return m.updateReturningSettings(key)
	}
	if m.stage == "returning-sync" {
		return m.updateReturningSync(key)
	}
	if m.stage == "returning-home" && m.activeTab == TabStatus && m.dashboard != nil {
		if m.searching {
			switch key.String() {
			case "esc":
				m.searching = false
				m.search.Blur()
				m.search.SetValue("")
				m.tabCursors[TabStatus] = 0
				m.message = "Filter cleared."
				return m, m.statusHighlight()
			case "enter":
				m.searching = false
				m.search.Blur()
				return m, m.statusHighlight()
			default:
				var cmd tea.Cmd
				m.search, cmd = m.search.Update(key)
				m.tabCursors[TabStatus] = 0
				return m, tea.Batch(cmd, m.statusHighlight())
			}
		}
		return m.updateStatusTab(key)
	}
	if m.stage == "returning-inspect" {
		switch key.String() {
		case "esc", "enter":
			m.resetReturning()
		case "j", "down", "J":
			m.viewport.ScrollDown(1)
		case "k", "up", "K":
			m.viewport.ScrollUp(1)
		}
		return m, nil
	}
	// Loading must not trap the user: tabs, Esc and q always work, even
	// mid-scan. Stale scans are discarded by inventoryGeneration.
	switch key.String() {
	case "1":
		return m.switchTab(TabStatus)
	case "2":
		return m.switchTab(TabTrack)
	case "3":
		return m.switchTab(TabAdopt)
	case "4":
		return m.switchTab(TabUntrack)
	case "5":
		return m.switchTab(TabSync)
	case "6", "W":
		return m.switchTab(TabSettings)
	case "q":
		if m.setupCancel != nil {
			m.setupCancel()
		}
		return m, tea.Quit
	case "esc":
		m.resetReturning()
		return m, nil
	}
	if m.loading {
		return m, nil
	}
	if key.String() == "esc" {
		m.resetReturning()
		return m, nil
	}
	switch key.String() {
	case "j", "down":
		if m.stage == "returning-remote-confirm" {
			m.confirmChoice = 1
		} else {
			m.returningChoice = min(len(returningActions)-1, m.returningChoice+1)
		}
	case "k", "up":
		if m.stage == "returning-remote-confirm" {
			m.confirmChoice = 0
		} else {
			m.returningChoice = max(0, m.returningChoice-1)
		}
	case "enter":
		if m.stage == "returning-remote-confirm" {
			if m.confirmChoice == 0 {
				m.resetReturning()
				return m, nil
			}
			if pending, _ := recoveryState(m.engine); pending {
				m.stage = "returning-home"
				m.activeTab = TabStatus
				m.confirmChoice = 0
				m.message = "Recovery pending — run dot recovery"
				return m, nil
			}
			e, ok := m.engine.(ReturningEngine)
			if !ok {
				m.err = fmt.Errorf("returning-user remote API unavailable")
				return m, nil
			}
			m.loading = true
			operation := m.returningOperation
			return m, func() tea.Msg {
				return returningMsg{err: e.Remote(m.setupCtx(), operation, true), remote: true}
			}
		}
		switch m.returningChoice {
		case 0:
			if m.setupCancel != nil {
				m.setupCancel()
			}
			return m, tea.Quit
		case 1, 2:
			m.mode = "track"
			if m.returningChoice == 2 {
				m.mode = "adopt"
			}
			m.stage = ""
			m.inventoryGeneration++
			m.loading = true
			m.cursor = 0
			m.inventoryReady = false
			return m, m.loadInventory
		case 3, 4, 5:
			operation := map[int]string{3: "status", 4: "diff", 5: "scope"}[m.returningChoice]
			e, ok := m.engine.(ReturningEngine)
			if !ok {
				m.err = fmt.Errorf("returning-user inspection API unavailable")
				return m, nil
			}
			m.returningOperation = operation
			m.loading = true
			return m, func() tea.Msg {
				text, err := e.Inspect(m.setupCtx(), operation)
				return returningMsg{generation: m.tabGeneration, text: text, err: err}
			}
		case 6, 7:
			m.returningOperation = "pull"
			if m.returningChoice == 7 {
				m.returningOperation = "push"
			}
			if pending, _ := recoveryState(m.engine); pending {
				m.message = "Recovery pending — run dot recovery"
				return m, nil
			}
			m.stage = "returning-remote-confirm"
			m.viewport.GotoTop()
			m.confirmChoice = 0
			m.err = nil
		}
	}
	return m, nil
}
func (m model) renderReturning() string {
	if m.hasStatusWorkspace() {
		return m.renderStatusTab()
	}
	if m.stage == "returning-settings" {
		return m.renderSettings()
	}
	if m.stage == "returning-sync" {
		return m.renderSync()
	}
	if m.stage == "returning-inspect" {
		footer := "j/k scroll · J/K scroll · Enter/Esc back · q quit=Finish without changes"
		if m.err != nil {
			footer = "Inspection failed: " + m.err.Error() + "\n" + footer
		}
		return m.renderTextWorkspace("Read-only inspection", m.viewport.GetContent(), footer)
	}
	if m.stage == "returning-remote-confirm" {
		action := "Pull fast-forward only; dirty work is preserved/refused. No adoption."
		if m.returningOperation == "push" {
			action = "Push already-existing commits only; no automatic commit or apply."
		}
		view := strings.ToUpper(m.returningOperation) + " · explicit remote operation\n" + action + "\n"
		for i, label := range []string{"Cancel", "Confirm explicit " + m.returningOperation} {
			mark := "  "
			if i == m.confirmChoice {
				mark = "> "
			}
			view += "\n" + mark + label
		}
		if m.err != nil {
			view += "\n" + m.err.Error()
		}
		if m.loading {
			view += "\nWorking…"
		}
		return m.renderTextWorkspace("Confirm Git operation", view, "j/k move · Enter confirm · Esc cancel · q quit=Finish without changes")
	}
	var top strings.Builder
	top.WriteString(m.tabBarBlock() + "\n")
	top.WriteString("What next?\n")
	top.WriteString(m.machineCard() + "\n")
	top.WriteString("No files preselected; inspection is not consent.\n\n")
	dash := m.renderDashboard()
	var menu strings.Builder
	extraLines := 5
	if m.currentMachine.ID != "" {
		extraLines++
	}
	if dash != "" {
		extraLines += strings.Count(dash, "\n")
	}
	extraLines += 2
	if m.loading || m.err != nil || m.message != "" {
		extraLines++
	}
	availableRows := max(1, m.height-extraLines)
	start := 0
	end := len(returningActions)
	if availableRows < len(returningActions) {
		start = max(0, min(m.returningChoice-availableRows+1, len(returningActions)-availableRows))
		if m.returningChoice < start {
			start = m.returningChoice
		}
		end = min(len(returningActions), start+availableRows)
	}
	for i := start; i < end; i++ {
		mark := "  "
		if i == m.returningChoice {
			mark = "> "
		}
		fmt.Fprintf(&menu, "%s%s\n", mark, returningActions[i])
	}
	if m.returningChoice >= 0 && m.returningChoice < len(returningDescriptions) {
		menu.WriteString("\n" + muted.Render(returningDescriptions[m.returningChoice]) + "\n")
	}
	var foot strings.Builder
	if m.loading {
		foot.WriteString("\nInspecting…")
	} else if m.err != nil {
		foot.WriteString("\n" + m.err.Error())
	} else if m.message != "" {
		foot.WriteString("\n" + m.message)
	}
	foot.WriteString("\n1-6 tabs · W settings · j/k move · J/K scroll · Enter select · q quit=Finish without changes")
	var b strings.Builder
	b.WriteString(top.String())
	b.WriteString(dash)
	pad := 0
	if h := m.height; h > 0 {
		if n := h - (strings.Count(top.String()+dash+menu.String()+foot.String(), "\n") + 1); n > 0 {
			pad = n
		}
	}
	b.WriteString(strings.Repeat("\n", pad))
	b.WriteString(menu.String())
	b.WriteString(foot.String())
	return b.String()
}

// renderDashboard is the always-visible status table: tracked files with
// target state, adoptable/diff notes, or start-tracking guidance when empty.
// Nothing renders for engines without StatusEngine data.
func (m model) renderDashboard() string {
	if m.dashboardLoading && m.dashboard == nil && m.dashboardErr == nil {
		return "Loading status…\n\n"
	}
	if m.dashboardErr != nil {
		return "Status unavailable: " + safeTerminal(m.dashboardErr.Error()) + "\n\n"
	}
	if m.dashboard == nil {
		return ""
	}
	d := m.dashboard
	var b strings.Builder
	b.WriteString(initSectionHead.Render("Tracked files") + "\n")
	if len(d.Managed) == 0 {
		b.WriteString("Nothing tracked yet.\n")
		b.WriteString("Track files to version HOME files into the repository, or Adopt files to bring the repository here.\n")
	} else {
		showN := len(d.Managed)
		if h := m.height; h > 0 {
			showN = max(1, min(showN, h-20))
		}
		b.WriteString(dashboardTable(d.Managed[:showN]) + "\n")
		if len(d.Managed) > showN {
			fmt.Fprintf(&b, "+%d more — open Status for the full list.\n", len(d.Managed)-showN)
		}
		if d.Diffs > 0 {
			fmt.Fprintf(&b, "%d file(s) differ — open Diff to review.\n", d.Diffs)
		}
	}
	if d.Adoptable > 0 {
		fmt.Fprintf(&b, "%d file(s) available — Adopt files applies them here.\n", d.Adoptable)
	}
	b.WriteString("\n")
	return b.String()
}

func dashboardState(raw string) string {
	switch raw {
	case "", "unchanged":
		return depOK.Render("✓ current")
	case "changed":
		return depAttention.Render("● changed")
	case "missing":
		return depAttention.Render("● missing")
	default:
		return initHint.Render("○ " + raw)
	}
}

// dashboardTable renders the tracked-files overview with bubbles table.
// Full per-file detail lives one Enter away on Status; the overview may
// truncate very long paths with ….
func dashboardTable(rows []DashboardEntry) string {
	fileW := 7
	for _, row := range rows {
		fileW = max(fileW, lipgloss.Width(row.Path))
	}
	fileW = min(52, fileW) + 2
	tableRows := make([]table.Row, len(rows))
	for i, row := range rows {
		tableRows[i] = table.Row{depTool.Render(row.Path), dashboardState(row.State)}
	}
	t := table.New(
		table.WithColumns([]table.Column{
			{Title: "FILE", Width: fileW},
			{Title: "STATE", Width: 12},
		}),
		table.WithRows(tableRows),
		table.WithFocused(false),
		table.WithHeight(1+len(tableRows)),
		table.WithWidth(fileW+12),
		table.WithStyles(table.Styles{
			Header:   initTableHead,
			Cell:     lipgloss.NewStyle(),
			Selected: lipgloss.NewStyle(),
		}),
	)
	return t.View()
}
func RunReturning(ctx context.Context, e Engine) error {
	_, err := tea.NewProgram(NewReturningModelContext(ctx, e), tea.WithContext(ctx)).Run()
	return err
}

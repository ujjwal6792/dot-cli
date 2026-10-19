package tui

import (
	"context"
	"fmt"
	"strings"

	tea "charm.land/bubbletea/v2"
	"charm.land/lipgloss/v2"
)

// Tab IDs for the returning-user adapter. The adapter is view-only: it
// reuses the shared Engine boundary (InventoryMode/Snapshot/Highlight/
// Plan/Preview/Apply) plus the optional ReturningEngine (Inspect/Remote)
// and StatusEngine (DashboardStatus) seams. No policy lives here.
const (
	TabStatus   = 1
	TabTrack    = 2
	TabAdopt    = 3
	TabUntrack  = 4
	TabSync     = 5
	TabSettings = 6
)

var tabTitles = map[int]string{
	TabStatus:   "Status & Diff",
	TabTrack:    "Track",
	TabAdopt:    "Adopt",
	TabUntrack:  "Stop-Tracking",
	TabSync:     "Git Sync",
	TabSettings: "Settings",
}

func tabTitle(id int) string {
	if title, ok := tabTitles[id]; ok {
		return title
	}
	return tabTitles[TabStatus]
}

func (m model) currentTab() int {
	if !m.returning {
		switch m.mode {
		case "track":
			return TabTrack
		case "adopt":
			return TabAdopt
		case "untrack":
			return TabUntrack
		}
	}
	if m.activeTab < TabStatus || m.activeTab > TabSettings {
		return TabStatus
	}
	return m.activeTab
}

// tabBar shares the prototype palette across all six tabs. Narrow terminals
// wrap complete labels onto another row without changing keyboard bindings.
func (m model) tabBar() string {
	width := max(1, m.width)
	var lines []string
	line := ""
	for id := TabStatus; id <= TabSettings; id++ {
		label := fmt.Sprintf(" [%d] %s ", id, tabTitle(id))
		style := lipgloss.NewStyle().Foreground(lipgloss.Color(AppTheme.Muted)).Background(lipgloss.Color(AppTheme.Mantle))
		if id == m.currentTab() {
			style = style.Foreground(lipgloss.Color(AppTheme.Accent)).Background(lipgloss.Color(AppTheme.Selection)).Bold(true)
		}
		part := style.Render(label)
		if lipgloss.Width(line+part) > width && line != "" {
			lines = append(lines, cellLine(line, width))
			line = ""
		}
		line += part
	}
	lines = append(lines, cellLine(line, width))
	return strings.Join(lines, "\n")
}

func (m model) tabBarBlock() string { return m.tabBar() }

// returningChrome wraps browser-stage overlays (review/confirm/metadata/
// browser) with the tab bar while returning. Navigation never authorizes
// writes; the bar is chrome, not consent.
func (m model) returningChrome(body string) string {
	if m.returning {
		title := "Review"
		switch m.stage {
		case "confirm":
			title = "Final Confirmation · Cancel Default"
		case "metadata", "logical", "template":
			title = "Edit metadata"
		case "preset":
			title = "Compose profiles"
		}
		wrapped := lipgloss.NewStyle().Width(max(1, m.width-4)).Render(body)
		return m.workspaceFrame(panel(title, fitHeight(wrapped, max(1, m.workspaceHeight()-2)), m.width, m.workspaceHeight(), true), false)
	}
	return m.renderOverlay(body)
}

// browserChrome prefixes the tab bar on the file-browser panes while
// returning. Other sessions render exactly as before.
func (m model) browserChrome(body string) string {
	if m.returning {
		return m.workspaceFrame(panel("Completed", body, m.width, m.workspaceHeight(), true), false)
	}
	return body
}

// invalidatePlan marks any derived plan stale. Selection, edits, retargets,
// profile composition, and direction/mode changes all revoke freshness and
// clear the separate unsafe/desktop/backup acknowledgments. Tab switches are
// browsing and preserve the plan.
func (m *model) invalidatePlan() {
	m.planFresh = false
	m.unsafeAck, m.desktopAck, m.backupAck = false, false, false
}

// assignmentLabel renders machine assignment in human terms only. Raw
// JSON/OS-name internals never reach the screen.
func assignmentLabel(id string) string {
	if id == "" || id == "all" {
		return "All machines"
	}
	return id + " only"
}

// refreshTabViewport stores the current tab pane in the viewport on the
// update path, so J/K scroll and cursor markers act on stored lines.
// Render-time SetContent only repaints and preserves the offset.
func (m *model) refreshTabViewport() {
	m.resizeTextViewport()
	m.viewport.SoftWrap = true
	switch m.stage {
	case "returning-sync":
		m.viewport.SetContent(m.syncBody())
	case "returning-settings":
		m.viewport.SoftWrap = false
		m.viewport.SetContent(m.settingsBody())
	}
}

// recoveryPendingChecker is implemented by engines that can report an
// interrupted operation without performing one.
type recoveryPendingChecker interface {
	RecoveryPending() (bool, error)
}

// recoveryState reports interrupted-operation state for display and for
// refusing remote work. Unknown engines fail closed: pull/push still refuse
// at the engine with an explicit message.
func recoveryState(e Engine) (pending bool, detail string) {
	if c, ok := e.(recoveryPendingChecker); ok {
		blocked, err := c.RecoveryPending()
		if err != nil {
			return false, "recovery record requires review: " + safeTerminal(err.Error())
		}
		if blocked {
			return true, "pending — run dot recovery; Pull/Push blocked"
		}
		return false, "none pending"
	}
	return false, "state unavailable here; remote operations refuse while interrupted"
}

// configPathEngine exposes the engine's path configuration for the
// Settings pane. Setup engines inherit it through the native adapter.
type configPathEngine interface {
	ConfigPaths() (source, home, state, tracker string)
}

func configPaths(e Engine) (source, home, state, tracker string) {
	if c, ok := e.(configPathEngine); ok {
		return c.ConfigPaths()
	}
	return "", "", "", ""
}

// setupCheckEngine reports fresh-install state without changing anything:
// "new-user" opens the wizard, "old-user" does not. The TUI never imports
// the setup package; engines that embed the setup seams satisfy this.
type setupCheckEngine interface {
	Check(context.Context) (string, error)
}

type settingsDepsMsg struct {
	result DependencyResult
	err    error
}

type settingsCheckMsg struct {
	state string
	err   error
}

// switchTab navigates between the six returning tabs. Switching is browsing:
// basket, metadata edits, profile composition, and plan freshness are
// preserved. File tabs reload inventory read-only for their direction.
func (m model) switchTab(id int) (tea.Model, tea.Cmd) {
	if id < TabStatus || id > TabSettings {
		m.message = "Unknown tab."
		return m, nil
	}
	if m.activeTab >= TabTrack && m.activeTab <= TabUntrack {
		m.tabCursors[m.activeTab] = m.cursor
		if path := m.browserDirs[m.activeTab]; m.dirPending[path] {
			// A cancelled lazy load must leave a reachable folder to retry,
			// rather than an empty current directory with no selected child.
			m.browserDirs[m.activeTab] = parentPath(path)
			m.tabCursors[m.activeTab] = 0
		}
	}
	m.cancelPreview()
	m.dirPending = map[string]bool{}
	m.highlight = HighlightResult{}
	m.inventoryGeneration++ // pending folder/inventory replies belong to the old tab
	m.tabGeneration++
	m.dashboardLoading = false
	m.loading = false
	m.activeTab = id
	m.settingsArmed = false
	m.message = ""
	m.err = nil
	switch id {
	case TabStatus:
		m.stage = "returning-home"
		m.returningChoice = 0
		m.viewport.GotoTop()
		if cmd := m.dashboardCommand(); cmd != nil {
			m.dashboardLoading = true
			m.dashboardErr = nil
			return m, cmd
		}
		return m, nil
	case TabTrack, TabAdopt, TabUntrack:
		want := map[int]string{TabTrack: "track", TabAdopt: "adopt", TabUntrack: "untrack"}[id]
		m.cursor = m.tabCursors[id]
		if want == "untrack" {
			m.message = "Stop tracking reviews private source/scope only; live files unchanged."
		}
		// Repeat visits reuse the cached inventory: no rescan, instant tabs.
		// Stale states (apply, rescan, source change) clear invCache.
		if cached, ok := m.invCache[want]; ok && m.mode == want {
			m.entries = cached.Entries
			m.profiles = cached.Profiles
			m.machines = cached.Machines
			m.currentMachine = cached.Machine
			m.currentOS = cached.OS
			m.stage = ""
			m.inventoryReady = true
			m.loading = false
			m.filter()
			return m, m.highlightCurrent()
		}
		if cached, ok := m.invCache[want]; ok {
			m.mode = want
			m.stage = ""
			m.currentOS = cached.OS
			m.currentMachine = cached.Machine
			m.entries = cached.Entries
			m.profiles = cached.Profiles
			m.machines = cached.Machines
			m.inventoryReady = true
			m.loading = false
			m.filter()
			return m, m.highlightCurrent()
		}
		if m.mode == want && m.inventoryReady && m.stage == "" {
			m.filter()
			return m, m.highlightCurrent()
		}
		m.mode = want
		m.stage = ""
		m.entries, m.visible = nil, nil
		m.inventoryErr = nil
		m.inventoryGeneration++
		m.inventoryReady = false
		m.loading = true
		return m, m.loadInventory
	case TabSync:
		m.stage = "returning-sync"
		m.syncChoice = 0
		m.refreshTabViewport()
		m.viewport.GotoTop()
		return m, nil
	case TabSettings:
		m.stage = "returning-settings"
		m.initSettings()
		m.settingsSearch.Blur()
		m.settingsFocus = 0
		m.refreshTabViewport()
		m.viewport.GotoTop()
		return m, nil
	}
	return m, nil
}

// returningTabKey handles tab switches for every returning stage. Text
// entry (search, metadata editor, template/logical editors) and the package
// browser keep their keys.
func (m model) returningTabKey(key tea.KeyPressMsg) (tea.Model, tea.Cmd, bool) {
	if m.packageView || m.searching || m.stage == "returning-settings" && m.settingsSearch.Focused() {
		return m, nil, false
	}
	switch m.stage {
	case "metadata", "template", "logical":
		return m, nil, false
	}
	switch key.String() {
	case "tab", "shift+tab":
		delta := 1
		if key.String() == "shift+tab" {
			delta = -1
		}
		id := (m.currentTab()-TabStatus+delta+6)%6 + TabStatus
		next, cmd := m.switchTab(id)
		return next, cmd, true
	case "1":
		next, cmd := m.switchTab(TabStatus)
		return next, cmd, true
	case "2":
		next, cmd := m.switchTab(TabTrack)
		return next, cmd, true
	case "3":
		next, cmd := m.switchTab(TabAdopt)
		return next, cmd, true
	case "4":
		next, cmd := m.switchTab(TabUntrack)
		return next, cmd, true
	case "5":
		next, cmd := m.switchTab(TabSync)
		return next, cmd, true
	case "6", "W":
		next, cmd := m.switchTab(TabSettings)
		return next, cmd, true
	}
	return m, nil, false
}

func (m model) settingsDepsCommand() tea.Cmd {
	e, ok := m.engine.(SetupEngine)
	if !ok {
		return nil
	}
	return func() tea.Msg {
		result, err := e.InitDependencies(m.setupCtx())
		return settingsDepsMsg{result, err}
	}
}

func (m model) settingsCheckCommand() tea.Cmd {
	e, ok := m.engine.(setupCheckEngine)
	if !ok {
		return nil
	}
	return func() tea.Msg {
		state, err := e.Check(m.setupCtx())
		return settingsCheckMsg{state, err}
	}
}

func (m model) settingsResultDeps(msg settingsDepsMsg) (tea.Model, tea.Cmd) {
	m.loading = false
	if msg.err != nil {
		m.message = "Dependency re-verification failed: " + safeTerminal(msg.err.Error())
		return m, nil
	}
	m.dependencies = msg.result
	m.message = "Dependencies re-verified; nothing installed."
	m.refreshTabViewport()
	return m, nil
}

func (m model) settingsResultCheck(msg settingsCheckMsg) (tea.Model, tea.Cmd) {
	if msg.err != nil {
		m.message = "Setup check (offline/read-only) failed: " + safeTerminal(msg.err.Error())
		return m, nil
	}
	m.settingsCheckState = msg.state
	m.message = "Setup check (offline/read-only): " + safeTerminal(msg.state) + ". Wizard runs on fresh install only."
	m.refreshTabViewport()
	return m, nil
}

// settingsSourceResult keeps source replacement inside Settings view-only:
// backup-plan is read-only; replacement itself needs the exact backup path
// and never runs from here without it.
func (m model) settingsSourceResult(msg sourceMsg) (tea.Model, tea.Cmd) {
	m.loading = false
	m.settingsArmed = false
	if msg.err != nil {
		m.message = "Source backup plan failed: " + safeTerminal(msg.err.Error())
		m.refreshTabViewport()
		return m, nil
	}
	if msg.action == "backup-plan" {
		m.sourceBackup = msg.result.Backup
		if msg.result.Backup != "" {
			m.message = "Backup plan ready: " + safeTerminal(msg.result.Backup) + ". Original preserved; replacement needs exact-path confirmation."
		} else {
			m.message = "Backup plan ready. Original preserved; replacement needs exact-path confirmation."
		}
		m.refreshTabViewport()
		return m, nil
	}
	m.message = "Source check complete; nothing replaced."
	return m, nil
}

func (m model) performSettingsAction(key tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	switch key.String() {
	case "esc":
		next, cmd := m.switchTab(TabStatus)
		return next, cmd
	case "j", "down", "J":
		m.viewport.ScrollDown(1)
	case "k", "up", "K":
		m.viewport.ScrollUp(1)
	case "D":
		if cmd := m.settingsDepsCommand(); cmd != nil {
			m.loading = true
			m.message = "Re-verifying dependencies…"
			return m, cmd
		}
		m.message = "Dependency verification unavailable in this build."
	case "T":
		if cmd := m.settingsCheckCommand(); cmd != nil {
			m.message = "Checking setup state (offline/read-only)…"
			return m, cmd
		}
		m.message = "Tracker is a local record of this connection; never commits or pushes."
	case "R":
		if !m.settingsArmed {
			m.settingsArmed = true
			m.message = "Replace is backup-first: the checkout moves to backup-<UTC>/checkout, .git/dirty/untracked preserved, never deleted. URL must be credential-free HTTPS/SSH. Press R again to plan the backup, Esc cancels."
			m.refreshTabViewport()
			return m, nil
		}
		m.settingsArmed = false
		if _, ok := m.engine.(SourceEngine); !ok {
			m.message = "Source replacement unavailable in this build."
			return m, nil
		}
		m.loading = true
		m.message = "Planning timestamped backup…"
		return m, func() tea.Msg {
			result, err := m.engine.(SourceEngine).InitSource(m.setupCtx(), map[string]any{"action": "backup-plan"})
			return sourceMsg{action: "backup-plan", result: result, err: err}
		}
	}
	return m, nil
}

// confirmSync enters the Cancel-default remote confirmation after refusing
// recovery-blocked work with an explicit message.
func (m model) confirmSync(operation string) (tea.Model, tea.Cmd) {
	if pending, _ := recoveryState(m.engine); pending {
		m.message = "Recovery pending — run dot recovery"
		m.stage = "returning-sync"
		return m, nil
	}
	m.returningOperation = operation
	m.stage = "returning-remote-confirm"
	m.viewport.GotoTop()
	m.confirmChoice = 0
	m.err = nil
	return m, nil
}

func (m model) updateReturningSync(key tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	switch key.String() {
	case "esc":
		next, cmd := m.switchTab(TabStatus)
		return next, cmd
	case "j", "down":
		m.syncChoice = 1
		m.refreshTabViewport()
	case "k", "up":
		m.syncChoice = 0
		m.refreshTabViewport()
	case "J":
		m.viewport.ScrollDown(1)
	case "K":
		m.viewport.ScrollUp(1)
	case "p":
		return m.confirmSync("pull")
	case "P":
		return m.confirmSync("push")
	case "enter":
		operation := "pull"
		if m.syncChoice == 1 {
			operation = "push"
		}
		return m.confirmSync(operation)
	}
	return m, nil
}

// syncBody builds the Git Sync pane. The viewport content is refreshed on
// the update path (switchTab and sync actions) so J/K scroll against stored
// lines; render-time SetContent only repaints the viewing copy.
func (m model) syncBody() string {
	var b strings.Builder
	b.WriteString("GIT SYNC · Explicit Gates\n")
	pending, detail := recoveryState(m.engine)
	fmt.Fprintf(&b, "Recovery: %s\n", detail)
	if pending {
		b.WriteString("Pull/Push refused until recovery completes.\n")
	}
	b.WriteString("Pull is fast-forward only and refuses dirty work; push sends existing commits only.\n")
	b.WriteString("Pull: git pull --ff-only --no-rebase origin <branch>\n")
	b.WriteString("Push: git push origin HEAD:refs/heads/<branch>\n\n")
	for i, label := range []string{"Pull repository (explicit)", "Push existing commits (explicit)"} {
		mark := "  "
		if i == m.syncChoice {
			mark = "> "
		}
		fmt.Fprintf(&b, "%s%s\n", mark, label)
	}
	if m.err != nil {
		b.WriteString("\n" + safeTerminal(m.err.Error()) + "\n")
	}
	return b.String()
}

func (m model) renderSync() string {
	return m.renderTextWorkspace("Git Sync", m.syncBody(), "j/k choose · p pull · P push · Enter confirm · Esc back")
}

// machineCard is the immutable machine context: adoption source of truth.
// Identity never changes through Settings.
func (m model) machineCard() string {
	id, os := safeTerminal(m.currentMachine.ID), safeTerminal(m.currentMachine.OS)
	if id == "" {
		return "Machine: unknown · immutable\nOpen Track/Adopt to load inventory; identity stays local, nothing adopted."
	}
	return fmt.Sprintf("Machine: %s [%s] · immutable", id, os)
}

// statusFiles projects tracked files into immediate HOME-relative children.
// An explicit search returns full-path matches across the tracked catalog.
func (m model) statusFiles() []DashboardEntry {
	if m.dashboard == nil {
		return nil
	}
	return dashboardRows(m.dashboard.Managed, m.browserDirs[TabStatus], m.search.Value(), m.hidden)
}

func statusCursor(m *model) int {
	n := len(m.statusFiles())
	if n == 0 {
		return -1
	}
	c := m.tabCursors[TabStatus]
	if c < 0 {
		c = 0
	}
	if c >= n {
		c = n - 1
	}
	m.tabCursors[TabStatus] = c
	return c
}

// The browser leads; a narrow left column holds the selected file's preview.
func (m model) renderStatusTab() string { return m.renderStatusWorkspace() }

// updateStatusTab handles Tab 1 keys. Navigation never authorizes writes;
// Enter opens folders; p/Enter on a file refreshes its inline preview.
func (m model) updateStatusTab(key tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	switch key.String() {
	case " ", "space":
		m.statusExpanded = !m.statusExpanded
		m.resizePreview()
		return m, nil
	case "pgdown", "pgup", "home", "end":
		n := len(m.statusFiles())
		m.tabCursors[TabStatus] = pagedCursor(statusCursor(&m), n, m.statusListLimit(), key.String())
		return m, m.statusHighlight()
	case "j", "down":
		if n := len(m.statusFiles()); n > 0 {
			m.tabCursors[TabStatus] = min(n-1, statusCursor(&m)+1)
		}
		return m, m.statusHighlight()
	case "k", "up":
		if n := len(m.statusFiles()); n > 0 {
			m.tabCursors[TabStatus] = max(0, statusCursor(&m)-1)
		}
		return m, m.statusHighlight()
	case "J":
		m.resizePreview()
		m.previewViewport.ScrollDown(1)
		return m, nil
	case "K":
		m.resizePreview()
		m.previewViewport.ScrollUp(1)
		return m, nil
	case "/":
		m.searching = true
		m.search.SetValue("")
		m.search.Focus()
		return m, nil
	case "esc":
		if m.statusExpanded {
			m.statusExpanded = false
			m.resizePreview()
			return m, nil
		}
		m.searching = false
		m.search.Blur()
		m.search.SetValue("")
		m.tabCursors[TabStatus] = 0
		m.message = "Filter cleared."
		return m, m.statusHighlight()
	case "h", "left", "backspace":
		path := m.browserDirs[TabStatus]
		m.browserDirs[TabStatus] = parentPath(path)
		m.search.SetValue("")
		m.tabCursors[TabStatus] = 0
		for i, e := range m.statusFiles() {
			if e.Path == path {
				m.tabCursors[TabStatus] = i
				break
			}
		}
		return m, m.statusHighlight()
	case "l", "right", "enter":
		files := m.statusFiles()
		if cur := statusCursor(&m); cur >= 0 && files[cur].Kind == "directory" {
			m.browserDirs[TabStatus] = files[cur].Path
			m.tabCursors[TabStatus] = 0
			m.search.SetValue("")
		}
		return m, m.statusHighlight()
	case "p":
		return m, m.statusHighlight()
	case "H":
		m.hidden = !m.hidden
		m.tabCursors[TabStatus] = 0
		return m, m.statusHighlight()
	case "r":
		m.dashboardLoading = true
		return m, m.dashboardCommand()
	}
	return m, nil
}

// statusHighlight debounces the single-file preview and invalidates older
// requests before they can overwrite the current detail pane.
func (m *model) statusHighlight() tea.Cmd {
	files := m.statusFiles()
	cur := statusCursor(m)
	m.listOffsets[TabStatus] = visibleStart(cur, len(files), m.statusListLimit(), m.listOffsets[TabStatus])
	if cur < 0 || cur >= len(files) {
		m.cancelPreview()
		m.highlight = HighlightResult{}
		return nil
	}
	if e := files[cur]; e.Kind == "directory" {
		m.cancelPreview()
		m.highlightPath = e.Path
		m.highlight = HighlightResult{Status: "directory", Summary: folderSummary(e.Tracked, e.Changed, e.Missing)}
		m.previewViewport.GotoTop()
		return nil
	}
	return m.queuePreview("adopt", files[cur].Path, nil)
}

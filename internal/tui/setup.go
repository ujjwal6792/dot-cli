package tui

import (
	"context"
	"io"
	"strings"

	"charm.land/bubbles/v2/table"
	"charm.land/bubbles/v2/textinput"
	tea "charm.land/bubbletea/v2"
	"charm.land/lipgloss/v2"
)

// TerminalStreams are supplied by tea.Exec after releasing the terminal. Never
// open /dev/tty or capture sudo/password answers inside the model.
type TerminalStreams struct {
	In       io.Reader
	Out, Err io.Writer
}
type DependencyRow struct{ Tool, Command, Link, Problem, Version string }
type DependencyResult struct {
	Ready   bool
	Manager string
	Rows    []DependencyRow
	Plan    map[string]any
}
type SetupResult struct {
	Exists bool
	Plan   map[string]any
	Review string
}
type SetupEngine interface {
	InitDependencies(context.Context) (DependencyResult, error)
	InstallDependencies(context.Context, map[string]any, string, TerminalStreams) error
	InitTracker(context.Context, map[string]any) (SetupResult, error)
	Complete(context.Context) error
}

// PackageTerminalEngine keeps package installation separate from config Apply.
type PackageTerminalEngine interface {
	InstallPackages(context.Context, map[string]any, string, TerminalStreams) error
}

type setupMsg struct {
	action       string
	dependencies DependencyResult
	tracker      SetupResult
	err          error
}

func (m model) setupCtx() context.Context {
	if m.setupContext != nil {
		return m.setupContext
	}
	return context.Background()
}

func (m *model) setupCommand(action string, fields map[string]any) tea.Cmd {
	e, ok := m.engine.(SetupEngine)
	if !ok {
		m.message = "Native setup is unavailable; setup incomplete."
		return nil
	}
	m.loading = true
	m.message = ""
	return func() tea.Msg {
		out := setupMsg{action: action}
		switch action {
		case "dependencies":
			out.dependencies, out.err = e.InitDependencies(m.setupCtx())
		case "tracker-plan", "tracker-apply":
			request := map[string]any{"action": strings.TrimPrefix(action, "tracker-")}
			for k, v := range fields {
				request[k] = v
			}
			out.tracker, out.err = e.InitTracker(m.setupCtx(), request)
		case "complete":
			out.err = e.Complete(m.setupCtx())
		}
		return out
	}
}
func (m model) setupResult(msg setupMsg) (tea.Model, tea.Cmd) {
	m.loading = false
	if msg.err != nil {
		m.message = msg.err.Error()
		if msg.action == "tracker-plan" || msg.action == "tracker-apply" {
			m.stage = "tracker-edit"
			m.trackerPlan = nil
			m.trackerEditing = false
			return m, m.trackerName.Focus()
		}
		if msg.action == "complete" {
			m.stage = "setup-incomplete"
		}
		return m, nil
	}
	switch msg.action {
	case "dependencies":
		m.dependencies = msg.dependencies
		m.stage = "dependencies"
		m.viewport.SetContent(safeTerminal(dependencyTable(m.dependencies, false)))
		m.viewport.SoftWrap = true
		m.viewport.SetHeight(max(3, m.height-9))
		m.viewport.GotoTop()
	case "install":
		cmd := m.setupCommand("dependencies", nil)
		return m, cmd
	case "tracker-plan":
		m.trackerPlan = msg.tracker.Plan
		m.trackerReview = msg.tracker.Review
		m.stage = "tracker-confirm"
		m.confirmChoice = 0
		m.trackerEditing = false
		m.trackerName.Blur()
		m.trackerEmail.Blur()
		m.viewport.SetContent(sectionedTrackerReview(m.trackerReview))
		m.viewport.SetHeight(max(3, m.height-10))
		m.viewport.GotoTop()
	case "tracker-apply":
		cmd := m.setupCommand("complete", nil)
		return m, cmd
	case "complete":
		m.stage = "done"
		m.message = "Source, machine, dependencies and consented tracker connection verified.\nNo files adopted; no Git settings changed, commit or push."
	}
	return m, nil
}

type dependencyInstall struct {
	engine  SetupEngine
	ctx     context.Context
	plan    map[string]any
	streams TerminalStreams
}

func (p *dependencyInstall) SetStdin(r io.Reader)  { p.streams.In = r }
func (p *dependencyInstall) SetStdout(w io.Writer) { p.streams.Out = w }
func (p *dependencyInstall) SetStderr(w io.Writer) { p.streams.Err = w }
func (p *dependencyInstall) Run() error {
	return p.engine.InstallDependencies(p.ctx, p.plan, inputValue(p.plan["id"]), p.streams)
}
func (m *model) startTracker() tea.Cmd {
	m.stage = "tracker-edit"
	m.trackerField = 0
	m.trackerName = textinput.New()
	m.trackerName.Prompt = "Name: "
	m.trackerName.Placeholder = "Git user.name, or tracker-only name"
	m.trackerName.CharLimit = 256
	m.trackerEmail = textinput.New()
	m.trackerEmail.Prompt = "Email: "
	m.trackerEmail.Placeholder = "Git user.email, or tracker-only email"
	m.trackerEmail.CharLimit = 256
	return m.setupCommand("tracker-plan", nil)
}
func (m model) updateTrackerInput(msg tea.Msg) (tea.Model, tea.Cmd) {
	var cmd tea.Cmd
	if m.trackerField == 0 {
		m.trackerName, cmd = m.trackerName.Update(msg)
	} else {
		m.trackerEmail, cmd = m.trackerEmail.Update(msg)
	}
	return m, cmd
}
func (m model) updateSetup(key tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	if key.String() == "ctrl+c" {
		return m, tea.Quit
	}
	if m.loading {
		return m, nil
	}
	if m.stage == "tracker-edit" {
		switch key.String() {
		case "esc":
			m.stage = "dependencies"
			m.dependenciesNext = "tracker-edit"
			m.message = ""
			return m, m.setupCommand("dependencies", nil)
		case "tab":
			m.trackerField = 1 - m.trackerField
			m.trackerName.Blur()
			m.trackerEmail.Blur()
			if m.trackerField == 0 {
				return m, m.trackerName.Focus()
			}
			return m, m.trackerEmail.Focus()
		case "ctrl+u":
			if m.trackerField == 0 {
				m.trackerName.SetValue("")
			} else {
				m.trackerEmail.SetValue("")
			}
			return m, nil
		case "enter":
			cmd := m.setupCommand("tracker-plan", map[string]any{"name": m.trackerName.Value(), "email": m.trackerEmail.Value()})
			return m, cmd
		}
		return m.updateTrackerInput(key)
	}
	if key.String() == "q" && !m.trackerEditing {
		return m, tea.Quit
	}
	switch m.stage {
	case "dependencies":
		switch key.String() {
		case "J":
			m.viewport.ScrollDown(1)
		case "K":
			m.viewport.ScrollUp(1)
		case "r":
			cmd := m.setupCommand("dependencies", nil)
			return m, cmd
		case "space", " ":
			m.stage = "setup-incomplete"
			m.message = "Required dependency step skipped. No installation authorized; setup incomplete."
		case "esc":
			m.stage = "machine"
			return m, m.machineCommand("suggest")
		case "enter":
			if m.dependencies.Ready {
				if m.dependenciesNext == "source" {
					return m.startSource("detect", nil)
				}
				if m.dependenciesNext == "tracker-edit" {
					m.stage = "tracker-edit"
					m.trackerField = 0
					m.trackerEditing = false
					return m, m.trackerName.Focus()
				}
				cmd := m.startTracker()
				return m, cmd
			}
			e, ok := m.engine.(SetupEngine)
			if !ok || m.dependencies.Plan == nil {
				m.message = "Refresh after installing required tools manually; setup incomplete."
				return m, nil
			}
			m.loading = true
			return m, tea.Exec(&dependencyInstall{engine: e, ctx: m.setupCtx(), plan: m.dependencies.Plan}, func(err error) tea.Msg { return setupMsg{action: "install", err: err} })
		}
	case "tracker-confirm":
		if m.trackerEditing {
			switch key.String() {
			case "esc":
				m.trackerEditing = false
				m.stage = "tracker-edit"
				m.trackerField = 0
				m.message = "Connection consent canceled. Back to identity editing — values kept, nothing written."
				return m, m.trackerName.Focus()
			case "tab":
				m.trackerField = 1 - m.trackerField
				m.trackerName.Blur()
				m.trackerEmail.Blur()
				if m.trackerField == 0 {
					return m, m.trackerName.Focus()
				}
				return m, m.trackerEmail.Focus()
			case "ctrl+u":
				if m.trackerField == 0 {
					m.trackerName.SetValue("")
				} else {
					m.trackerEmail.SetValue("")
				}
				return m, nil
			case "J":
				m.viewport.ScrollDown(1)
				return m, nil
			case "K":
				m.viewport.ScrollUp(1)
				return m, nil
			case "enter":
				m.trackerEditing = false
				cmd := m.setupCommand("tracker-plan", map[string]any{"name": m.trackerName.Value(), "email": m.trackerEmail.Value()})
				return m, cmd
			}
			return m.updateTrackerInput(key)
		}
		switch key.String() {
		case "j", "down":
			m.confirmChoice = 1
		case "k", "up":
			m.confirmChoice = 0
		case "J":
			m.viewport.ScrollDown(1)
		case "K":
			m.viewport.ScrollUp(1)
		case "esc":
			m.stage = "tracker-edit"
			m.trackerField = 0
			m.message = "Connection consent canceled. Back to identity editing — values kept, nothing written."
			return m, m.trackerName.Focus()
		case "e":
			m.trackerEditing = true
			m.trackerField = 0
			m.trackerName.Blur()
			m.trackerEmail.Blur()
			m.message = "Editing identity inline — Enter re-reviews, Esc goes back."
			return m, m.trackerName.Focus()
		case "enter":
			if m.confirmChoice == 0 {
				m.stage = "tracker-edit"
				m.trackerField = 0
				m.message = "Connection consent canceled. Back to identity editing — values kept, nothing written."
				return m, m.trackerName.Focus()
			}
			cmd := m.setupCommand("tracker-apply", map[string]any{"plan": m.trackerPlan, "approve": inputValue(m.trackerPlan["id"])})
			return m, cmd
		}
	case "setup-incomplete":
		if key.String() == "r" {
			cmd := m.setupCommand("dependencies", nil)
			return m, cmd
		}
		if key.String() == "esc" || key.String() == "enter" {
			return m, tea.Quit
		}
	}
	return m, nil
}
func (m model) renderSetup() string {
	if m.stage == "setup-incomplete" {
		return m.initShell("Dependencies", "Setup incomplete", "",
			"No successful setup claimed. Earlier confirmed steps are retained.",
			"", "r recheck dependencies · Enter/Esc quit", m.message)
	}
	if m.stage == "dependencies" {
		m.viewport.SetContent(safeTerminal(dependencyTable(m.dependencies, m.loading)))
		m.viewport.SoftWrap = true
		m.viewport.SetHeight(m.depHeight())
		hints := "r refresh · J/K scroll · Esc back · q quit"
		if !m.dependencies.Ready {
			hints = "Enter Install · Space skip\nNative prompts; no password capture.\n" + hints
		} else {
			hints = "Enter continue · Space stop\n" + hints
		}
		return m.initShell("Dependencies", "Dependencies", "", m.viewport.View(), "",
			hints, m.message)
	}
	if m.stage == "tracker-edit" {
		body := "Blank uses your Git identity when it is set.\nWithout Git identity, these tracker-only values are required.\nGit settings stay unchanged.\n\n" + m.trackerName.View() + "\n" + m.trackerEmail.View()
		return m.initShell("Tracker", "Tracker · local identity", "Edit freely — nothing is written until you confirm.", body, "",
			"Tab next field · Ctrl+U clear · Enter review · Esc back", m.message)
	}
	m.viewport.SetHeight(m.trackerHeight())
	m.viewport.SetContent(sectionedTrackerReview(m.trackerReview))
	status := m.message
	if m.loading {
		status = "Verifying exact consented connection…"
	}
	choices := []string{"Cancel", "Confirm tracker and connection"}
	question := initTitle.Render("Create this tracker and record the connection?")
	editable := ""
	hints := "j/k choose · J/K scroll · e edit here · Enter confirm · Esc back"
	if m.trackerEditing {
		editable = "\n" + m.trackerName.View() + "\n" + m.trackerEmail.View() + "\n"
		hints = "Tab switch field · Ctrl+U clear · J/K scroll · Enter re-review · Esc back"
	}
	return m.initShell("Tracker", "Tracker · review connection consent", "Verify the identity and paths below, then decide.",
		m.viewport.View(), question+editable+"\n"+initChoiceList(choices, m.confirmChoice),
		hints, status)
}

// sectionedTrackerReview groups the engine's review text into labeled
// sections — connection facts, tracker file content, scope limits — without
// changing any wording. Keys render bold with values plain; full lines stay
// contiguous for assertions. Unknown lines pass through unstyled.
func sectionedTrackerReview(review string) string {
	var conn, file, scope []string
	for _, line := range strings.Split(safeTerminal(review), "\n") {
		trimmed := strings.TrimSpace(line)
		if trimmed == "" {
			continue
		}
		if kvKey(trimmed) != "" {
			conn = append(conn, renderKVLine(trimmed))
			continue
		}
		if strings.HasPrefix(trimmed, "Create ") || strings.HasPrefix(trimmed, "Compatible existing tracker") {
			file = append(file, strong.Render(trimmed))
			continue
		}
		if strings.HasPrefix(trimmed, "Persist ") {
			scope = append(scope, trimmed)
			continue
		}
		if tomlKey(trimmed) != "" || trimmed == "[data]" {
			file = append(file, renderKVLineCustom(trimmed, " = "))
			continue
		}
		file = append(file, trimmed)
	}
	var b strings.Builder
	if len(conn) > 0 {
		b.WriteString(initSectionHead.Render("Connection") + "\n")
		b.WriteString(strings.Join(conn, "\n") + "\n")
	}
	if len(file) > 0 {
		b.WriteString("\n" + initSectionHead.Render("Tracker file") + "\n")
		b.WriteString(strings.Join(file, "\n") + "\n")
	}
	if len(scope) > 0 {
		b.WriteString("\n" + initSectionHead.Render("Scope") + "\n")
		b.WriteString(strings.Join(scope, "\n"))
	}
	return strings.TrimSuffix(b.String(), "\n")
}

// kvKey matches "Machine:", "Source:", ... summary lines.
func kvKey(line string) string {
	if i := strings.Index(line, ":"); i > 0 {
		key := strings.TrimSpace(line[:i])
		for _, known := range []string{"Machine", "Source", "Origin", "HOME", "Tracker", "Connection"} {
			if key == known {
				return key
			}
		}
	}
	return ""
}

// tomlKey matches `key = "value"` lines, returning the key.
func tomlKey(line string) string {
	if i := strings.Index(line, "="); i > 0 {
		key := strings.TrimSpace(line[:i])
		if key != "" && !strings.ContainsAny(key, " \t\"") {
			return key
		}
	}
	return ""
}

func renderKVLine(line string) string {
	i := strings.Index(line, ":")
	return strong.Render(strings.TrimSpace(line[:i])+":") + " " + strings.TrimSpace(line[i+1:])
}

func renderKVLineCustom(line, sep string) string {
	i := strings.Index(line, sep)
	if i < 0 {
		return strong.Render(line)
	}
	return strong.Render(strings.TrimSpace(line[:i])) + line[i:]
}

var (
	initTableHead = lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color(AppTheme.Muted))
	depTool       = lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color(AppTheme.Accent))
	depOK         = lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color(AppTheme.Success))
	depAttention  = lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color(AppTheme.Status))
)

// depTable renders the PACKAGE/VERSION/STATUS summary with bubbles table.
// Detail lines (exact command, link, notes) stay outside it, untruncated.
func depTable(rows []DependencyRow) string {
	toolW, versionW, statusW := 7, 7, 6
	type cell struct {
		tool, version, status string
		ok                    bool
		note                  string
	}
	cells := make([]cell, len(rows))
	tableRows := make([]table.Row, len(rows))
	for i, row := range rows {
		toolW = max(toolW, lipgloss.Width(row.Tool))
		version := row.Version
		if version == "" {
			version = "—"
		}
		versionW = max(versionW, lipgloss.Width(version))
		short, note, ok := depStatus(row.Problem)
		statusW = max(statusW, lipgloss.Width(short))
		cells[i] = cell{row.Tool, version, short, ok, note}
	}
	for i, c := range cells {
		status := depAttention.Render(c.status)
		if c.ok {
			status = depOK.Render(c.status)
		}
		version := initHint.Render(c.version)
		if rows[i].Version != "" {
			version = initBody.Render(c.version)
		}
		tableRows[i] = table.Row{depTool.Render(c.tool), version, status}
	}
	t := table.New(
		table.WithColumns([]table.Column{
			{Title: "PACKAGE", Width: toolW + 2},
			{Title: "VERSION", Width: versionW + 2},
			{Title: "STATUS", Width: statusW + 2},
		}),
		table.WithRows(tableRows),
		table.WithFocused(false),
		table.WithHeight(1+len(tableRows)),
		table.WithWidth(toolW+versionW+statusW+6),
		table.WithStyles(table.Styles{
			Header:   initTableHead,
			Cell:     lipgloss.NewStyle(),
			Selected: lipgloss.NewStyle(),
		}),
	)
	var b strings.Builder
	b.WriteString(t.View() + "\n")
	for i, row := range rows {
		if row.Problem == "" {
			continue
		}
		if row.Command != "" {
			b.WriteString("  " + initBody.Render(row.Command) + "\n")
		}
		b.WriteString("  " + initHint.Render(row.Link) + "\n")
		if cells[i].note != "" {
			b.WriteString("  " + initHint.Render("note: "+cells[i].note) + "\n")
		}
	}
	return strings.TrimSuffix(b.String(), "\n")
}

// depStatus fits the status column; long problems move to a note line so the
// columns stay aligned.
func depStatus(problem string) (short, note string, ok bool) {
	if problem == "" {
		return "✓ supported", "", true
	}
	if lipgloss.Width(problem)+2 <= 16 {
		return "● " + problem, "", false
	}
	return "● see note", problem, false
}

func dependencyTable(d DependencyResult, loading bool) string {
	var b strings.Builder
	b.WriteString("Only Git and chezmoi are required. Existing installs are never overwritten.\n")
	if loading {
		b.WriteString("\nChecking required tools…\n")
	} else if d.Ready {
		b.WriteString("\nGit and chezmoi verified. Nothing to install.\n")
	}
	if len(d.Rows) > 0 {
		b.WriteString("\n" + depTable(d.Rows) + "\n")
	}
	return strings.TrimSuffix(b.String(), "\n")
}

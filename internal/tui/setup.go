package tui

import (
	"context"
	"fmt"
	"io"
	"strings"

	"charm.land/bubbles/v2/textinput"
	tea "charm.land/bubbletea/v2"
)

// TerminalStreams are supplied by tea.Exec after releasing the terminal. Never
// open /dev/tty or capture sudo/password answers inside the model.
type TerminalStreams struct {
	In       io.Reader
	Out, Err io.Writer
}
type DependencyRow struct{ Tool, Command, Link, Problem string }
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
		m.trackerName.Blur()
		m.trackerEmail.Blur()
		m.viewport.SetContent(safeTerminal(m.trackerReview))
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
			return m, tea.Quit
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
	if key.String() == "q" {
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
		case "esc", "space", " ":
			m.stage = "setup-incomplete"
			m.message = "Required dependency step skipped. No installation authorized; setup incomplete."
		case "enter":
			if m.dependencies.Ready {
				if m.dependenciesNext == "source" {
					return m.startSource("detect", nil)
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
			m.message = "Connection consent canceled. No tracker or connection written."
			return m, m.trackerName.Focus()
		case "enter":
			if m.confirmChoice == 0 {
				m.stage = "tracker-edit"
				m.trackerField = 0
				m.message = "Connection consent canceled. No tracker or connection written."
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
	progress := "Welcome → Source → Machine → Dependencies → Tracker → Done\n\n"
	if m.width < 80 {
		progress = "Setup · " + map[string]string{"dependencies": "4/6 Dependencies", "tracker-edit": "5/6 Tracker", "tracker-confirm": "5/6 Tracker", "setup-incomplete": "Incomplete"}[m.stage] + "\n\n"
	}
	if m.stage == "setup-incomplete" {
		return progress + "Setup incomplete\n" + m.message + "\nNo successful setup claimed. Earlier confirmed steps are retained.\n\nr recheck dependencies · Enter/Esc quit"
	}
	if m.stage == "dependencies" {
		var b strings.Builder
		m.viewport.SetContent(safeTerminal(dependencyTable(m.dependencies, m.loading)))
		m.viewport.SoftWrap = true
		m.viewport.SetHeight(max(3, m.height-9))
		body := progress + "Dependencies\n" + m.viewport.View() + "\n\n"
		b.Reset()
		b.WriteString(body)
		if !m.dependencies.Ready {
			b.WriteString("Enter Install · Space skip\nNative prompts; no password capture.\n")
		} else {
			b.WriteString("Enter continue · Space stop\n")
		}
		b.WriteString("r refresh · J/K scroll table · q quit\n" + m.message)
		return b.String()
	}
	if m.stage == "tracker-edit" {
		return progress + "Tracker · local identity\nUse existing Git identity, or enter tracker-only fallback values. Git settings stay unchanged.\n" + m.trackerName.View() + "\n" + m.trackerEmail.View() + "\n\nTab next field · Ctrl+U clear · Enter review · Esc quit\n" + m.message
	}
	m.viewport.SetHeight(max(3, m.height-10))
	m.viewport.SetContent(safeTerminal(m.trackerReview))
	cancel, confirm := "> ", "  "
	if m.confirmChoice == 1 {
		cancel, confirm = "  ", "> "
	}
	status := m.message
	if m.loading {
		status = "Verifying exact consented connection…"
	}
	return progress + "Tracker · review connection consent\n" + m.viewport.View() + "\n\n" + cancel + "Cancel\n" + confirm + "Confirm tracker and connection\n\nj/k choose · J/K scroll · Enter confirm · Esc cancel\n" + status
}
func dependencyTable(d DependencyResult, loading bool) string {
	var b strings.Builder
	b.WriteString("Only Git and chezmoi are required. Existing installs are never overwritten.\n\nTool · Manager command · Official link\n")
	if loading {
		b.WriteString("Checking required tools…\n")
	} else if d.Ready {
		b.WriteString("Git and chezmoi verified. Nothing to install.\n")
	}
	for _, row := range d.Rows {
		fmt.Fprintf(&b, "%s · %s\n  %s\n  %s\n", row.Tool, row.Problem, row.Command, row.Link)
	}
	return b.String()
}

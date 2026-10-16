package tui

import (
	"context"
	"fmt"
	"io"
	"strings"

	"charm.land/bubbles/v2/textinput"
	tea "charm.land/bubbletea/v2"
)

// SourceEngine extends the shared engine boundary without requiring source
// provisioning from engines used solely for the configuration browser.
type SourceEngine interface {
	InitSource(context.Context, map[string]any) (SourceResult, error)
}
type SourceTerminalEngine interface {
	InitSourceTerminal(context.Context, map[string]any, TerminalStreams) (SourceResult, error)
}
type sourceClone struct {
	engine  SourceTerminalEngine
	ctx     context.Context
	request map[string]any
	streams TerminalStreams
	result  SourceResult
}

func (p *sourceClone) SetStdin(r io.Reader)  { p.streams.In = r }
func (p *sourceClone) SetStdout(w io.Writer) { p.streams.Out = w }
func (p *sourceClone) SetStderr(w io.Writer) { p.streams.Err = w }
func (p *sourceClone) Run() error {
	var err error
	p.result, err = p.engine.InitSourceTerminal(p.ctx, p.request, p.streams)
	return err
}

type SourceResult struct {
	NeedsGit bool           `json:"needs_git,omitempty"`
	Exists   bool           `json:"exists"`
	Valid    bool           `json:"valid"`
	Backup   string         `json:"backup,omitempty"`
	Plan     map[string]any `json:"plan,omitempty"`
	Error    string         `json:"error,omitempty"`
}

type sourceMsg struct {
	action string
	result SourceResult
	err    error
}

func (m *model) sourceCommand(action string, fields map[string]any) tea.Cmd {
	e, ok := m.engine.(SourceEngine)
	if !ok {
		m.message = "Source setup is unavailable in this build."
		return nil
	}
	request := map[string]any{"action": action}
	for k, v := range fields {
		request[k] = v
	}
	m.loading = true
	m.message = ""
	if action == "clone" {
		native, ok := m.engine.(SourceTerminalEngine)
		if !ok {
			m.loading = false
			m.message = "Native Git terminal handoff is unavailable; nothing cloned."
			return nil
		}
		process := &sourceClone{engine: native, ctx: m.setupCtx(), request: request}
		return tea.Exec(process, func(err error) tea.Msg { return sourceMsg{action: action, result: process.result, err: err} })
	}
	return func() tea.Msg {
		result, err := e.InitSource(m.setupCtx(), request)
		return sourceMsg{action, result, err}
	}
}

func (m model) startSource(action string, fields map[string]any) (tea.Model, tea.Cmd) {
	cmd := m.sourceCommand(action, fields)
	return m, cmd
}

func (m model) sourceResult(msg sourceMsg) (tea.Model, tea.Cmd) {
	m.loading = false
	if msg.err != nil {
		m.message = msg.err.Error()
		if msg.action == "continue" {
			m.stage = "source-recovery"
			m.sourceError = msg.err.Error()
			m.sourceChoice = 0
			return m, nil
		}
		// A failed Git operation may have left a checkout. Detect it before
		// offering another write, so recovery goes through Continue/Replace.
		if msg.action == "clone" || msg.action == "new" {
			message := m.message
			cmd := m.sourceCommand("detect", nil)
			m.message = message
			return m, cmd
		}
		return m, nil
	}
	switch msg.action {
	case "detect":
		if msg.result.NeedsGit {
			m.stage = "dependencies"
			m.dependenciesNext = "source"
			cmd := m.setupCommand("dependencies", nil)
			return m, cmd
		}
		m.sourceChoice = 0
		if msg.result.Exists {
			if !msg.result.Valid && msg.result.Error != "" {
				m.stage = "source-recovery"
				m.sourceError = msg.result.Error
			} else {
				m.stage = "source-existing"
			}
		} else {
			m.stage = "source-choice"
		}
	case "backup-plan":
		m.sourcePlan = msg.result.Plan
		m.sourceBackup = msg.result.Backup
		m.sourceChoice = 0
		m.stage = "source-confirm"
		m.viewport.SetContent(safeTerminal(backupReview(m.sourceBackup)))
		m.viewport.SoftWrap = true
		m.viewport.SetHeight(max(3, m.height-7))
		m.viewport.GotoTop()
	case "replace":
		m.stage = "source-choice"
		m.sourceChoice = 0
		m.message = "Original checkout preserved at " + msg.result.Backup
	case "continue", "clone", "new":
		m.stage = "machine"
		m.newMachineForm("")
		cmd := m.machineCommand("suggest")
		return m, cmd
	}
	return m, nil
}

func (m model) updateInit(key tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	if key.String() == "ctrl+c" {
		if m.setupCancel != nil {
			m.setupCancel()
		}
		return m, tea.Interrupt
	}
	if key.String() == "q" && m.stage != "machine" && m.stage != "source-url" && m.stage != "tracker-edit" && m.stage != "source-recovery" && m.stage != "source-diagnostics" {
		if m.setupCancel != nil {
			m.setupCancel()
		}
		return m, tea.Quit
	}
	if m.loading {
		return m, nil
	}
	if m.stage == "machine" {
		return m.updateMachine(key)
	}
	if m.stage == "source-diagnostics" {
		switch key.String() {
		case "esc", "enter", "q":
			m.stage = "source-recovery"
			return m, nil
		}
		return m, nil
	}
	if m.stage == "source-confirm" {
		if key.String() == "J" {
			m.viewport.ScrollDown(1)
			return m, nil
		}
		if key.String() == "K" {
			m.viewport.ScrollUp(1)
			return m, nil
		}
	}
	if m.stage == "dependencies" || strings.HasPrefix(m.stage, "tracker-") || m.stage == "setup-incomplete" {
		return m.updateSetup(key)
	}
	if m.stage == "source-url" {
		switch key.String() {
		case "esc":
			m.sourceInput.SetValue("")
			m.stage = "source-choice"
			return m, nil
		case "enter":
			url := m.sourceInput.Value()
			m.sourceInput.SetValue("")
			return m.startSource(m.sourceAction, map[string]any{"url": url})
		}
		var cmd tea.Cmd
		m.sourceInput, cmd = m.sourceInput.Update(key)
		return m, cmd
	}
	switch key.String() {
	case "q", "esc":
		if m.stage == "source-confirm" || m.stage == "source-recovery" {
			m.stage = "source-existing"
			m.sourceChoice = 0
			return m, nil
		}
		if key.String() == "esc" && (m.stage == "source-existing" || m.stage == "source-choice") {
			m.stage = "welcome"
			m.message = ""
			return m, nil
		}
		return m, tea.Quit
	case "j", "down":
		maxChoice := 1
		if m.stage == "source-recovery" {
			maxChoice = 3
		}
		m.sourceChoice = min(maxChoice, m.sourceChoice+1)
	case "k", "up":
		m.sourceChoice = max(0, m.sourceChoice-1)
	case "enter":
		switch m.stage {
		case "welcome":
			return m.startSource("detect", nil)
		case "done":
			m.initMode = false
			m.returning = true
			m.resetReturning()
			if cmd := m.dashboardCommand(); cmd != nil {
				m.dashboardLoading = true
				m.dashboardErr = nil
				return m, cmd
			}
			return m, nil
		case "source-existing":
			if m.sourceChoice == 0 {
				return m.startSource("continue", nil)
			}
			return m.startSource("backup-plan", nil)
		case "source-recovery":
			switch m.sourceChoice {
			case 0:
				m.stage = "source-diagnostics"
				return m, nil
			case 1:
				return m.startSource("continue", nil)
			case 2:
				m.stage = "source-choice"
				m.sourceChoice = 0
				return m, nil
			case 3:
				return m.startSource("backup-plan", nil)
			}
		case "source-confirm":
			if m.sourceChoice == 0 {
				m.stage = "source-existing"
				m.sourceChoice = 0
				return m, nil
			}
			return m.startSource("replace", map[string]any{"plan": m.sourcePlan})
		case "source-choice":
			m.sourceAction = "clone"
			if m.sourceChoice == 1 {
				m.sourceAction = "new"
			}
			m.sourceInput = textinput.New()
			m.sourceInput.CharLimit = 4096
			m.stage = "source-url"
			m.message = ""
			cmd := m.sourceInput.Focus()
			return m, cmd
		}
	}
	return m, nil
}

func (m model) renderSource() string {
	switch m.stage {
	case "source-existing":
		choices := []string{"Continue", "Replace with timestamped backup"}
		return m.initShell("Source", "Source · existing checkout", "",
			"Continue checks the Git root and contract.\nReplace moves the whole checkout, as-is, to a timestamped backup beside it.\nNothing is deleted; the exact path is shown next for confirmation.",
			initChoiceList(choices, m.sourceChoice),
			"j/k move · Enter select · Esc back · Ctrl+C quit", m.message)
	case "source-recovery":
		entry, actual, supported, summary := parseContractError(m.sourceError)
		var body strings.Builder
		body.WriteString("The repository's dot-contract or manifest does not satisfy compatibility requirements.\nInvalid applies remain blocked; metadata is never silently rewritten.")
		if entry != "" {
			fmt.Fprintf(&body, "\n\n  Entry:            %s", entry)
		}
		if actual != "" {
			fmt.Fprintf(&body, "\n  Actual value:     %s", actual)
		}
		if supported != "" {
			fmt.Fprintf(&body, "\n  Supported values: %s", supported)
		}
		if summary != "" && summary != m.sourceError {
			fmt.Fprintf(&body, "\n  Issue:            %s", summary)
		}
		body.WriteString("\n\nSelect an actionable recovery option:")
		choices := []string{
			"Diagnostics: view contract details & full error",
			"Retry: re-check contract and repository",
			"Change source: connect a different repository",
			"Replace: backup existing checkout and re-initialize",
		}
		return m.initShell("Source", "Source · Contract Compatibility Recovery", "",
			body.String(),
			initChoiceList(choices, m.sourceChoice),
			"j/k choose recovery action · Enter select · Esc back · Ctrl+C quit", m.message)
	case "source-diagnostics":
		entry, actual, supported, _ := parseContractError(m.sourceError)
		var b strings.Builder
		b.WriteString("Error:\n  " + m.sourceError + "\n")
		if entry != "" {
			fmt.Fprintf(&b, "\nEntry: %s", entry)
		}
		if actual != "" {
			fmt.Fprintf(&b, "\nValue: %s", actual)
		}
		fmt.Fprintf(&b, "\nSupported: %s", supported)
		b.WriteString("\n\nInvalid applies remain strictly blocked to prevent corruption.\nMetadata is preserved intact and never silently modified.")
		b.WriteString("\n\nRules: managed-shared / managed-local / managed-templated / managed-os-only; OS linux, darwin; Impact Safe, Unsafe.")
		return m.initShell("Source", "Source · Contract Diagnostics & Repair Guidance", "",
			b.String(), "",
			"Enter / Esc return to recovery choices · Ctrl+C quit", m.message)
	case "source-choice":
		choices := []string{"Existing user: clone your repository", "New user: local git init + remote add origin"}
		return m.initShell("Source", "Source · connect your own repository", "Clone brings history · New starts versioning here", "",
			initChoiceList(choices, m.sourceChoice),
			"j/k move · Enter select · Esc back · Ctrl+C quit", m.message)
	case "source-confirm":
		m.viewport.SetContent(safeTerminal(backupReview(m.sourceBackup)))
		m.viewport.SetHeight(m.sourceConfirmHeight())
		m.viewport.SoftWrap = true
		choices := []string{"Cancel", "Confirm backup and replace"}
		body := m.viewport.View()
		return m.initShell("Source", "Source · confirm backup", "",
			body, initChoiceList(choices, m.sourceChoice),
			"j/k choose · J/K scroll backup path · Enter confirm · Esc back", m.message)
	case "source-url":
		var body strings.Builder
		body.WriteString("Source · paste a credential-free HTTPS or SSH repository URL\nInput hidden to avoid displaying credentials: " + strings.Repeat("•", min(40, len(m.sourceInput.Value()))))
		if m.sourceAction == "new" {
			body.WriteString("\n\nCreate the empty repository on your platform yourself.\nOnly local git init and remote add; no hosting API, commit or push.")
		} else {
			body.WriteString("\n\nNative Git/SSH authentication follows terminal handoff. Dot never reads passwords.")
		}
		if m.loading {
			body.WriteString("\n\nWorking…")
		}
		return m.initShell("Source", "Source · repository URL", "", body.String(), "",
			"Enter connect · Esc back · Ctrl+C quit", m.message)
	}
	return m.initShell("Source", "Source", "", "", "", "j/k move · Enter select · Esc cancel · Ctrl+C quit", m.message)
}

func parseContractError(errStr string) (entry string, actual string, supported string, summary string) {
	summary = errStr
	supported = "managed-shared, managed-local, managed-templated, managed-os-only"

	if idx := strings.Index(errStr, "entry "); idx >= 0 {
		rest := errStr[idx+len("entry "):]
		parts := strings.SplitN(rest, ":", 2)
		entry = strings.TrimSpace(parts[0])
		if len(parts) > 1 {
			summary = strings.TrimSpace(parts[1])
		}
	}

	if strings.Contains(errStr, "unsupported portability") {
		if start := strings.Index(errStr, `unsupported portability "`); start >= 0 {
			valRest := errStr[start+len(`unsupported portability "`):]
			if end := strings.Index(valRest, `"`); end >= 0 {
				actual = valRest[:end]
			}
		}
		supported = "managed-shared, managed-local, managed-templated, managed-os-only"
	} else if strings.Contains(errStr, "os-only requires exactly one OS") {
		actual = "multiple OS configured with managed-os-only"
		supported = "exactly one OS (e.g. [\"linux\"] or [\"darwin\"])"
	} else if strings.Contains(errStr, "unsupported entry OS") {
		actual = "unsupported OS"
		supported = "linux, darwin"
	} else if strings.Contains(errStr, "Safe/Unsafe impact") {
		actual = "missing or invalid impact"
		supported = "Safe, Unsafe"
	} else if strings.Contains(errStr, "unsupported integration") {
		actual = "unsupported integration"
		supported = "git-config, gemini-skills, claude-skills, desktop"
	} else if strings.Contains(errStr, "source origin is not credential-free") {
		actual = "unsupported origin URL"
		supported = "credential-free HTTPS or SSH URL"
	}

	return entry, actual, supported, summary
}

func backupReview(path string) string {
	return "Confirm Replace · original checkout will be moved, never deleted\nBackup path:\n" + path
}

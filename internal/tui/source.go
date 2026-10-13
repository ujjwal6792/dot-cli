package tui

import (
	"context"
	"fmt"
	"strings"

	"charm.land/bubbles/v2/textinput"
	tea "charm.land/bubbletea/v2"
)

// SourceEngine extends the shared engine boundary without requiring source
// provisioning from engines used solely for the configuration browser.
type SourceEngine interface {
	InitSource(context.Context, map[string]any) (SourceResult, error)
}

type SourceResult struct {
	Exists bool           `json:"exists"`
	Valid  bool           `json:"valid"`
	Backup string         `json:"backup,omitempty"`
	Plan   map[string]any `json:"plan,omitempty"`
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
	return func() tea.Msg {
		result, err := e.InitSource(context.Background(), request)
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
		m.sourceChoice = 0
		if msg.result.Exists {
			m.stage = "source-existing"
		} else {
			m.stage = "source-choice"
		}
	case "backup-plan":
		m.sourcePlan = msg.result.Plan
		m.sourceBackup = msg.result.Backup
		m.sourceChoice = 0
		m.stage = "source-confirm"
	case "replace":
		m.stage = "source-choice"
		m.sourceChoice = 0
		m.message = "Original checkout preserved at " + msg.result.Backup
	case "continue", "clone", "new":
		m.stage = "machine"
		m.machineInput = textinput.New()
		m.machineInput.CharLimit = 128
		cmd := m.machineCommand("suggest")
		return m, cmd
	}
	return m, nil
}

func (m model) updateInit(key tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	if key.String() == "ctrl+c" {
		return m, tea.Quit
	}
	if m.loading {
		return m, nil
	}
	if m.stage == "machine" {
		return m.updateMachine(key)
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
		if m.stage == "source-confirm" {
			m.stage = "source-existing"
			m.sourceChoice = 0
			return m, nil
		}
		return m, tea.Quit
	case "j", "down":
		m.sourceChoice = 1
	case "k", "up":
		m.sourceChoice = 0
	case "enter":
		switch m.stage {
		case "welcome":
			return m.startSource("detect", nil)
		case "done":
			return m, tea.Quit
		case "source-existing":
			if m.sourceChoice == 0 {
				return m.startSource("continue", nil)
			}
			return m.startSource("backup-plan", nil)
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
	var title string
	var choices []string
	switch m.stage {
	case "source-existing":
		title = "Source · existing checkout\nContinue checks the Git root and contract; Replace preserves all files."
		choices = []string{"Continue", "Replace with timestamped backup"}
	case "source-choice":
		title = "Source · connect your own repository"
		choices = []string{"Existing user: clone your repository", "New user: local git init + remote add origin"}
	case "source-confirm":
		title = "Confirm Replace · original checkout will be moved, never deleted\nBackup path:\n" + m.sourceBackup
		choices = []string{"Cancel", "Confirm backup and replace"}
	case "source-url":
		title = "Source · paste a credential-free HTTPS or SSH repository URL\nInput hidden to avoid displaying credentials: " + strings.Repeat("•", min(40, len(m.sourceInput.Value())))
		if m.sourceAction == "new" {
			title += "\nCreate the empty repository on your platform yourself.\nOnly local git init and remote add; no hosting API, commit or push."
		}
		if m.loading {
			title += "\nWorking…"
		}
		return title + "\n\nEnter connect · Esc back · Ctrl+C quit\n" + m.message
	}
	for i, choice := range choices {
		mark := "  "
		if i == m.sourceChoice {
			mark = "> "
		}
		title += fmt.Sprintf("\n%s%s", mark, choice)
	}
	if m.loading {
		title += "\nWorking…"
	}
	return title + "\n\nj/k move · Enter select · Esc cancel · Ctrl+C quit\n" + m.message
}

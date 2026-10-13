package tui

import (
	"context"

	"charm.land/bubbles/v2/textinput"
	tea "charm.land/bubbletea/v2"
)

type MachineEngine interface {
	InitMachine(context.Context, map[string]any) (MachineResult, error)
}

type Machine struct {
	ID string `json:"id"`
	OS string `json:"os"`
}

type MachineResult struct {
	Machine Machine `json:"machine"`
}

type machineMsg struct {
	action string
	result MachineResult
	err    error
}

func (m *model) machineCommand(action string) tea.Cmd {
	e, ok := m.engine.(MachineEngine)
	if !ok {
		m.message = "Machine identity setup is unavailable in this build."
		return nil
	}
	request := map[string]any{"action": action}
	if action == "save" {
		request["id"] = m.machineInput.Value()
	}
	m.loading = true
	return func() tea.Msg {
		result, err := e.InitMachine(m.setupCtx(), request)
		return machineMsg{action, result, err}
	}
}

func (m model) machineResult(msg machineMsg) (tea.Model, tea.Cmd) {
	m.loading = false
	if msg.err != nil {
		m.message = msg.err.Error()
		return m, m.machineInput.Focus()
	}
	if msg.action == "save" {
		m.stage = "dependencies"
		m.machineInput.Blur()
		m.currentMachine = msg.result.Machine
		m.dependenciesNext = "tracker"
		m.message = "Local machine " + msg.result.Machine.ID + " saved; no files adopted or tracker created."
		cmd := m.setupCommand("dependencies", nil)
		return m, cmd
	}
	m.machineInput = textinput.New()
	m.machineInput.CharLimit = 128
	m.machineInput.SetValue(msg.result.Machine.ID)
	m.machineOS = msg.result.Machine.OS
	m.message = "Source connected and contract-checked. Identity stays local; nothing adopted."
	return m, m.machineInput.Focus()
}

func (m model) updateMachine(key tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	switch key.String() {
	case "esc", "ctrl+c":
		if m.setupCancel != nil {
			m.setupCancel()
		}
		return m, tea.Quit
	case "enter":
		action := "save"
		if m.machineInput.Value() == "" {
			action = "suggest"
		}
		cmd := m.machineCommand(action)
		return m, cmd
	case "ctrl+u":
		m.machineInput.SetValue("")
		return m, nil
	}
	var cmd tea.Cmd
	m.machineInput, cmd = m.machineInput.Update(key)
	return m, cmd
}

func (m model) renderMachine() string {
	status := m.message
	if m.loading {
		status = "Checking local machine identity…"
	}
	return "Machine identity · " + m.machineOS + "\nEdit the suggested id: lowercase a-z, 0-9 and underscore; unique locally.\n" + m.machineInput.View() + "\n\nEnter save local identity · Ctrl+U clear · Esc quit without saving\n" + status
}

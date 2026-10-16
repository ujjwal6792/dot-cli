package tui

import (
	"context"
	"fmt"
	"regexp"

	tea "charm.land/bubbletea/v2"
	"charm.land/huh/v2"
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
		request["id"] = m.machineForm.GetString("id")
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
		value := ""
		if m.machineForm != nil {
			value, _ = m.machineForm.GetFocusedField().GetValue().(string)
		}
		m.newMachineForm(value)
		return m, m.machineForm.Init()
	}
	if msg.action == "save" {
		m.stage = "dependencies"
		m.currentMachine = msg.result.Machine
		m.dependenciesNext = "tracker"
		m.message = "Local machine " + msg.result.Machine.ID + " saved; no files adopted or tracker created."
		cmd := m.setupCommand("dependencies", nil)
		return m, cmd
	}
	m.newMachineForm(msg.result.Machine.ID)
	m.machineOS = msg.result.Machine.OS
	m.message = "Source connected and contract-checked. Identity stays local; nothing adopted."
	return m, m.machineForm.Init()
}

var formMachineID = regexp.MustCompile(`^[a-z0-9_]{1,128}$`)

func (m *model) newMachineForm(value string) {
	m.machineForm = huh.NewForm(huh.NewGroup(huh.NewInput().Key("id").Title("Machine id").
		Value(&value).CharLimit(128).Validate(func(s string) error {
		if !formMachineID.MatchString(s) {
			return fmt.Errorf("Use lowercase a-z, 0-9 and underscore (1–128 characters)")
		}
		return nil
	}))).WithWidth(max(20, m.width-8)).WithShowHelp(true)
	// Embedded model only: never call Run or open a second program/prompt.
}

func (m model) updateMachineForm(msg tea.Msg) (tea.Model, tea.Cmd) {
	if m.machineForm == nil {
		return m, nil
	}
	_, cmd := m.machineForm.Update(msg)
	if m.machineForm.State == huh.StateCompleted {
		return m, m.machineCommand("save")
	}
	return m, cmd
}

func (m model) updateMachine(key tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	switch key.String() {
	case "esc":
		return m.startSource("detect", nil)
	case "ctrl+c":
		if m.setupCancel != nil {
			m.setupCancel()
		}
		return m, tea.Interrupt
	}
	return m.updateMachineForm(key)
}

func (m model) renderMachine() string {
	status := m.message
	if m.loading {
		status = "Checking local machine identity…"
	}
	form := ""
	if m.machineForm != nil {
		form = m.machineForm.View()
	}
	body := "Edit the suggested id; uniqueness is checked locally.\n\n" + form
	return m.initShell("Machine", "Machine identity · "+m.machineOS, "", body, "",
		"Enter save local identity · Ctrl+U clear · Esc back to source", status)
}

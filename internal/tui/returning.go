package tui

import (
	"context"
	"fmt"
	"strings"

	tea "charm.land/bubbletea/v2"
)

type ReturningEngine interface {
	Inspect(context.Context, string) (string, error)
	Remote(context.Context, string, bool) error
}
type returningMsg struct {
	text   string
	err    error
	remote bool
}

var returningActions = []string{"Finish without changes", "Browse / Track: HOME → repository", "Browse / Adopt: repository → target", "Status (machine-scoped)", "Diff / importable changes", "Approved scope", "Pull repository (explicit)", "Push existing commits (explicit)"}

func NewReturningModel(e Engine) tea.Model {
	return NewReturningModelContext(context.Background(), e)
}

func NewReturningModelContext(ctx context.Context, e Engine) tea.Model {
	m := NewModel(e, false, "track").(model)
	m.setupContext, m.setupCancel = context.WithCancel(ctx)
	m.returning = true
	m.stage = "returning-home"
	return m
}
func (m *model) resetReturning() {
	m.stage = "returning-home"
	m.returningChoice = 0
	m.confirmChoice = 0
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
	m.message = "Nothing selected. Browsing is not authorization."
}
func (m model) updateReturning(key tea.KeyPressMsg) (tea.Model, tea.Cmd) {
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
	if m.stage == "returning-inspect" {
		switch key.String() {
		case "esc", "enter":
			m.resetReturning()
		case "j", "down":
			m.viewport.ScrollDown(1)
		case "k", "up":
			m.viewport.ScrollUp(1)
		}
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
				return returningMsg{text: text, err: err}
			}
		case 6, 7:
			m.returningOperation = "pull"
			if m.returningChoice == 7 {
				m.returningOperation = "push"
			}
			m.stage = "returning-remote-confirm"
			m.confirmChoice = 0
			m.err = nil
		}
	}
	return m, nil
}
func (m model) renderReturning() string {
	if m.stage == "returning-inspect" {
		footer := "j/k scroll · Enter/Esc back · q quit"
		if m.err != nil {
			footer = "Inspection failed: " + m.err.Error() + "\n" + footer
		}
		return strings.ToUpper(m.returningOperation) + " · machine-scoped / offline\n" + m.viewport.View() + "\n" + footer
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
		return view + "\n\nj/k move · Enter confirm · Esc cancel"
	}
	var b strings.Builder
	b.WriteString("DOT · returning-user actions\n")
	extraLines := 4
	if m.currentMachine.ID != "" {
		fmt.Fprintf(&b, "Machine: %s · %s\n", safeTerminal(m.currentMachine.ID), safeTerminal(m.currentMachine.OS))
		extraLines++
	}
	b.WriteString("No files preselected; inspection is not consent.\n\n")
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
		fmt.Fprintf(&b, "%s%s\n", mark, returningActions[i])
	}
	if m.loading {
		b.WriteString("\nInspecting…")
	} else if m.err != nil {
		b.WriteString("\n" + m.err.Error())
	} else if m.message != "" {
		b.WriteString("\n" + m.message)
	}
	b.WriteString("\nj/k move · Enter select · q quit")
	return b.String()
}
func RunReturning(ctx context.Context, e Engine) error {
	_, err := tea.NewProgram(NewReturningModelContext(ctx, e), tea.WithContext(ctx)).Run()
	return err
}

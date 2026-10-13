package tui

import (
	"context"
	"errors"
	"strings"
	"testing"

	tea "charm.land/bubbletea/v2"
)

type sourceFixture struct {
	fixtureEngine
	exists bool
	calls  []string
}

type machineFailureFixture struct {
	sourceFixture
	saved string
}

func (f *machineFailureFixture) InitMachine(_ context.Context, request map[string]any) (MachineResult, error) {
	if request["action"] == "suggest" {
		return MachineResult{}, errors.New("machine: collision suffixes exhausted; choose a unique id")
	}
	f.saved = request["id"].(string)
	return MachineResult{Machine: Machine{ID: f.saved, OS: "darwin"}}, nil
}

func TestMachineSuggestionFailureStillAllowsEditing(t *testing.T) {
	e := &machineFailureFixture{sourceFixture: sourceFixture{exists: true}}
	m := sendAndRun(NewInitModel(e), "enter")
	m = sendAndRun(m, "enter")
	if !strings.Contains(m.View().Content, "choose a unique id") {
		t.Fatal(m.View().Content)
	}
	m, _ = m.Update(tea.PasteMsg{Content: "unique_machine"})
	m = sendAndRun(m, "enter")
	if e.saved != "unique_machine" || !strings.Contains(m.View().Content, "Done") {
		t.Fatal("suggestion failure prevented editing: " + m.View().Content)
	}
}

func TestSourcePendingOperationCannotBeSubmittedTwice(t *testing.T) {
	e := &sourceFixture{}
	m, cmd := NewInitModel(e).Update(key("enter"))
	if cmd == nil {
		t.Fatal("missing detection command")
	}
	_, duplicate := m.Update(key("enter"))
	if duplicate != nil {
		t.Fatal("submitted a second operation while detection pending")
	}
}

func (f *sourceFixture) InitSource(_ context.Context, request map[string]any) (SourceResult, error) {
	action := request["action"].(string)
	f.calls = append(f.calls, action)
	if action == "backup-plan" {
		return SourceResult{Exists: true, Backup: "/home/source.backup-20261014T120000.000000Z/checkout", Plan: map[string]any{"backup": "/home/source.backup-20261014T120000.000000Z/checkout"}}, nil
	}
	if action == "replace" {
		f.exists = false
	}
	return SourceResult{Exists: f.exists, Valid: f.exists}, nil
}

func TestInitWelcomeNavigatesToSourceWithoutAdoptionCalls(t *testing.T) {
	e := &sourceFixture{}
	m := NewInitModel(e)
	if cmd := m.Init(); cmd != nil {
		t.Fatal("welcome must render without inventory before choice")
	}
	if view := m.View().Content; !strings.Contains(view, "Welcome") {
		t.Fatalf("welcome page missing: %s", view)
	}
	m = sendAndRun(m, "enter")
	if view := m.View().Content; !strings.Contains(view, "Existing user") || !strings.Contains(view, "New user") {
		t.Fatalf("source choices missing after welcome: %s", view)
	}
	if e.plans != 0 || e.previews != 0 || e.applies != 0 {
		t.Fatal("welcome/done invoked engine plan/preview/apply")
	}
}

func TestSourceReplaceRequiresSecondConfirmationNamingBackup(t *testing.T) {
	e := &sourceFixture{exists: true}
	m := sendAndRun(NewInitModel(e), "enter")
	if view := m.View().Content; !strings.Contains(view, "Continue") || !strings.Contains(view, "Replace") {
		t.Fatal(view)
	}
	m = send(m, "j")
	m = sendAndRun(m, "enter")
	view := m.View().Content
	if !strings.Contains(view, "/home/source.backup-20261014T120000.000000Z/checkout") {
		t.Fatal(view)
	}
	if strings.Contains(strings.Join(e.calls, ","), "replace") {
		t.Fatal("navigation replaced checkout")
	}
	m = sendAndRun(m, "enter") // Cancel is the default.
	if strings.Contains(strings.Join(e.calls, ","), "replace") {
		t.Fatal("default confirmed replacement")
	}
	m = send(m, "j")
	m = sendAndRun(m, "enter")
	m = send(m, "j")
	m = sendAndRun(m, "enter")
	if strings.Join(e.calls, ",") != "detect,backup-plan,backup-plan,replace" {
		t.Fatal(e.calls)
	}
	if !strings.Contains(m.View().Content, "Existing user") {
		t.Fatal(m.View().Content)
	}
}

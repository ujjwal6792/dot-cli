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
	if e.saved != "unique_machine" || !strings.Contains(m.View().Content, "Dependencies") || strings.HasPrefix(m.View().Content, "Done\n") {
		t.Fatal("suggestion failure prevented editing: " + m.View().Content)
	}
}

func TestMachineFormValidatesBeforeSubmitting(t *testing.T) {
	e := &machineFailureFixture{sourceFixture: sourceFixture{exists: true}}
	m := sendAndRun(NewInitModel(e), "enter")
	m = sendAndRun(m, "enter")
	m, _ = m.Update(tea.PasteMsg{Content: "not-valid!"})
	m = sendAndRun(m, "enter")
	if e.saved != "" || !strings.Contains(m.View().Content, "lowercase") {
		t.Fatal("invalid machine form submitted: " + m.View().Content)
	}
}

func TestSourceInputUsesTerminalPasteWithoutClipboardHelpers(t *testing.T) {
	m := sendAndRun(NewInitModel(&sourceFixture{}), "enter")
	m = sendAndRun(m, "j")
	m = sendAndRun(m, "enter")
	next, cmd := m.Update(tea.KeyPressMsg{Code: 'v', Mod: tea.ModCtrl})
	if cmd != nil || !strings.Contains(next.View().Content, "terminal paste") {
		t.Fatal("source input attempted an external clipboard helper")
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

type sourceContractFailureFixture struct {
	sourceFixture
	continueCalls int
}

func (f *sourceContractFailureFixture) InitSource(_ context.Context, request map[string]any) (SourceResult, error) {
	action := request["action"].(string)
	f.calls = append(f.calls, action)
	if action == "continue" {
		f.continueCalls++
		return SourceResult{Exists: true, Valid: false}, errors.New(`source contract check failed: entry .bashrc: unsupported portability "invalid-port" (supported: managed-shared, managed-local, managed-templated, managed-os-only)`)
	}
	if action == "backup-plan" {
		return SourceResult{Exists: true, Backup: "/home/source.backup-20261014T120000.000000Z/checkout", Plan: map[string]any{"backup": "/home/source.backup-20261014T120000.000000Z/checkout"}}, nil
	}
	return SourceResult{Exists: true, Valid: false, Error: `source contract check failed: entry .bashrc: unsupported portability "invalid-port" (supported: managed-shared, managed-local, managed-templated, managed-os-only)`}, nil
}

func TestSourceContractIncompatibilityShowsRecoveryGuidanceAndActionableChoices(t *testing.T) {
	e := &sourceContractFailureFixture{sourceFixture: sourceFixture{exists: true}}
	// Welcome -> detect -> invalid detected so goes straight to source-recovery
	m := sendAndRun(NewInitModel(e), "enter")
	view := m.View().Content

	if !strings.Contains(view, "Contract Compatibility Recovery") {
		t.Fatalf("expected recovery view, got: %s", view)
	}
	if !strings.Contains(view, ".bashrc") {
		t.Fatalf("expected entry .bashrc in view, got: %s", view)
	}
	if !strings.Contains(view, "invalid-port") {
		t.Fatalf("expected actual value in view, got: %s", view)
	}
	if !strings.Contains(view, "managed-shared") || !strings.Contains(view, "managed-os-only") {
		t.Fatalf("expected supported values in view, got: %s", view)
	}
	if !strings.Contains(view, "Diagnostics") || !strings.Contains(view, "Retry") || !strings.Contains(view, "Change source") || !strings.Contains(view, "Replace") {
		t.Fatalf("expected all 4 recovery choices in view, got: %s", view)
	}

	// Choice 0: Diagnostics
	m = send(m, "enter")
	diagView := m.View().Content
	if !strings.Contains(diagView, "Contract Diagnostics & Repair Guidance") {
		t.Fatalf("expected diagnostics view, got: %s", diagView)
	}
	if !strings.Contains(diagView, "Invalid applies remain strictly blocked") || !strings.Contains(diagView, "Metadata is preserved intact") {
		t.Fatalf("expected safety policy in diagnostics, got: %s", diagView)
	}

	// Esc returns to recovery
	m = send(m, "esc")
	if !strings.Contains(m.View().Content, "Contract Compatibility Recovery") {
		t.Fatalf("expected return to recovery, got: %s", m.View().Content)
	}

	// Choice 1: Retry
	m = send(m, "j")
	m = sendAndRun(m, "enter")
	if e.continueCalls != 1 {
		t.Fatalf("expected 1 continue call, got %d", e.continueCalls)
	}
	if !strings.Contains(m.View().Content, "Contract Compatibility Recovery") {
		t.Fatalf("expected return to recovery on failed retry, got: %s", m.View().Content)
	}

	// Choice 2: Change source (move from 0 -> 1 -> 2)
	m = send(m, "j")
	m = send(m, "j")
	m = send(m, "enter")
	if !strings.Contains(m.View().Content, "Existing user") || !strings.Contains(m.View().Content, "New user") {
		t.Fatalf("expected source-choice after Change source, got: %s", m.View().Content)
	}

	// Test Choice 3 (Replace)
	e2 := &sourceContractFailureFixture{sourceFixture: sourceFixture{exists: true}}
	m2 := sendAndRun(NewInitModel(e2), "enter")
	m2 = send(m2, "j")
	m2 = send(m2, "j")
	m2 = send(m2, "j")
	m2 = sendAndRun(m2, "enter")
	if !strings.Contains(m2.View().Content, "/home/source.backup-20261014T120000.000000Z/checkout") {
		t.Fatalf("expected backup review after selecting Replace, got: %s", m2.View().Content)
	}
}

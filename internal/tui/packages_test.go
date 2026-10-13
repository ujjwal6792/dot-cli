package tui

import (
	"context"
	"fmt"
	"strings"
	"testing"

	tea "charm.land/bubbletea/v2"
)

type fixturePackages struct {
	fixtureEngine
	installs, saves int
	rows            []PackageRow
	saveErr         error
	text, savedText string
}

func (f *fixturePackages) Packages(_ context.Context, action string, request map[string]any, plan any, approve string) (PackageResult, error) {
	if action == "plan" {
		return PackageResult{Plan: map[string]any{"id": "fixture-package-plan"}, Review: "fixture-tmux · exact missing package"}, nil
	}
	if action == "apply" {
		f.installs++
		return PackageResult{}, nil
	}
	if action == "save" {
		f.saves++
		f.savedText, _ = request["text"].(string)
		if f.saveErr != nil {
			return PackageResult{}, f.saveErr
		}
	}
	rows := f.rows
	if rows == nil {
		rows = []PackageRow{{Manager: "brew", Kind: "brew", Name: "fixture-cli", Desired: true, Installed: true}, {Manager: "brew", Kind: "brew", Name: "fixture-tmux", Desired: true, Missing: true}}
	}
	text := f.text
	if text == "" {
		text = "brew \"fixture-cli\"\n"
	}
	return PackageResult{Platform: "darwin", Rows: rows, Sources: map[string]PackageSource{"brew": {Text: text, Digest: "fixture-digest"}}}, nil
}

func TestPackagesUneditedSavePreservesOpaqueUnknownText(t *testing.T) {
	text := "# fixture\r\nbrew \"fixture-cli\"\r\nsystem(\"fixture\u200dvalue\")\r\n"
	e := &fixturePackages{text: text}
	m := sendAndRun(NewModel(e, false, "track"), "P")
	m = send(m, "e")
	if strings.Contains(m.View().Content, "\u200d") {
		t.Fatal("format control reached terminal editor view")
	}
	next, cmd := m.Update(tea.KeyPressMsg(tea.Key{Code: 19}))
	if cmd == nil {
		t.Fatal("save unavailable")
	}
	next, _ = next.Update(cmd())
	if e.savedText != text {
		t.Fatal("unedited save silently changed private source bytes")
	}
	if strings.Contains(next.View().Content, "\u200d") {
		t.Fatal("format control reached terminal view")
	}
}

func TestPackagesStaleSaveShowsRecoveryWithoutDiscardingEdits(t *testing.T) {
	e := &fixturePackages{saveErr: fmt.Errorf("packages: stale source; reload before saving")}
	m := sendAndRun(NewModel(e, false, "track"), "P")
	m = send(m, "e")
	// Use the actual terminal control key, not printable shortcut text.
	next, cmd := m.Update(tea.KeyPressMsg(tea.Key{Code: 19}))
	if cmd == nil {
		t.Fatal("save command unavailable")
	}
	next, _ = next.Update(cmd())
	view := next.View().Content
	if !strings.Contains(view, "stale source; reload before saving") || !strings.Contains(view, "PRIVATE PACKAGE SOURCE") {
		t.Fatalf("stale save hid editor or error: %s", view)
	}
	if e.installs != 0 {
		t.Fatal("saving installed packages")
	}
}

func TestPackagesRefreshAfterDeselectionHandlesEmptyState(t *testing.T) {
	e := &fixturePackages{}
	for i := 0; i < 40; i++ {
		e.rows = append(e.rows, PackageRow{Manager: "brew", Name: fmt.Sprintf("fixture-%d", i), Desired: true, Missing: true})
	}
	m := sendAndRun(NewModel(e, false, "track"), "P")
	for i := 0; i < 35; i++ {
		m = send(m, "j")
	}
	e.rows = []PackageRow{}
	m = sendAndRun(m, "r")
	if !strings.Contains(m.View().Content, "Press e to supply a private list") {
		t.Fatal("empty-state editing guidance absent")
	}
	if e.installs != 0 {
		t.Fatal("deselection installed or uninstalled")
	}
}

func TestPackagesInstallRequiresIndependentReviewAndDefaultCancel(t *testing.T) {
	e := &fixturePackages{}
	m := sendAndRun(NewModel(e, false, "track"), "P")
	m = sendAndRun(m, "i")
	if !strings.Contains(m.View().Content, "fixture-tmux · exact missing package") {
		t.Fatal("missing review")
	}
	m = send(m, "a")
	if !strings.Contains(m.View().Content, "> Cancel") {
		t.Fatal("default is not cancel")
	}
	m = sendAndRun(m, "enter")
	if e.installs != 0 {
		t.Fatal("default consent installed")
	}
	m = sendAndRun(m, "i")
	m = send(m, "a")
	m = send(m, "j")
	_, cmd := m.Update(key("enter"))
	if cmd == nil {
		t.Fatal("explicit consent did not return suspended-terminal installation command")
	}
	if e.installs != 0 {
		t.Fatal("installation ran before terminal release")
	}
}

func TestPackagesNavigationIsDistinctAndNeverInstalls(t *testing.T) {
	e := &fixturePackages{}
	m := NewModel(e, false, "track")
	m = sendAndRun(m, "P")
	view := m.View().Content
	for _, label := range []string{"Packages", "fixture-cli", "fixture-tmux", "missing", "Install missing"} {
		if !strings.Contains(view, label) {
			t.Fatalf("missing %q: %s", label, view)
		}
	}
	m = send(m, "e")
	if !strings.Contains(m.View().Content, "PRIVATE PACKAGE SOURCE") {
		t.Fatal("editor unavailable")
	}
	m = send(m, "esc")
	m = sendAndRun(m, "P")
	if !strings.Contains(m.View().Content, "Dotfiles") {
		t.Fatal("did not return to dotfiles")
	}
	if e.installs != 0 || e.plans != 0 || e.applies != 0 {
		t.Fatal("navigation installed or planned configs")
	}
}

func TestPackagesLoadingCanBeQuitWithoutConsent(t *testing.T) {
	e := &fixturePackages{}
	m := NewModel(e, false, "track")
	m, _ = m.Update(key("P")) // leave the read-only status request pending
	_, cmd := m.Update(key("q"))
	if cmd == nil {
		t.Fatal("quit ignored during read-only status loading")
	}
	if e.installs != 0 {
		t.Fatal("quit installed packages")
	}
}

func TestPackagesNavigationDiscardsPendingConfigurationReview(t *testing.T) {
	e := &fixturePackages{}
	e.inventory = Inventory{Entries: []Entry{{Path: "fixture.conf", Kind: "file", Status: "eligible", Metadata: map[string]any{"destination": "fixture.conf", "os": []string{"darwin"}, "portability": "managed-shared", "impact": "Safe"}}}}
	m := NewModel(e, false, "track")
	m, _ = m.Update(m.Init()())
	m = sendAndRun(m, " ")
	m, pending := m.Update(key("p"))
	if pending == nil {
		t.Fatal("fixture did not prepare config review")
	}
	m, _ = m.Update(key("P"))
	_, late := m.Update(pending())
	if late != nil {
		t.Fatal("late config response started another review while in Packages")
	}
	if e.installs != 0 || e.applies != 0 {
		t.Fatal("navigation applied a plan")
	}
}

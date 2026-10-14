package tui

import (
	"context"
	"os"
	"os/exec"
	"path/filepath"
	"testing"

	tea "charm.land/bubbletea/v2"

	"github.com/ujjwal6792/dots-cli/internal/native"
)

func realNative(t *testing.T) (*NativeEngine, native.Config) {
	t.Helper()
	root, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	c := native.Config{Source: filepath.Join(root, "source"), Home: filepath.Join(root, "home"), State: filepath.Join(root, "state"), OS: "linux"}
	for _, p := range []string{c.Source, c.Home, filepath.Join(c.Source, "home")} {
		if err := os.MkdirAll(p, 0700); err != nil {
			t.Fatal(err)
		}
	}
	for p, body := range map[string]string{"dot-contract.json": `{"protocol":1,"engine_api":{"min":1,"max":1},"manifest_version":1,"approval_version":3,"manifest":"manifest.json","home_source":"home","required_adapters":["intent","scope"]}`, "manifest.json": `{"version":1,"entries":{"prefs":{"source":"home/prefs","destination":"prefs","os":["linux"],"portability":"managed-shared","impact":"Safe"}},"profiles":{"base":["prefs"]}}`, "home/prefs": "real bytes\n"} {
		if err := os.WriteFile(filepath.Join(c.Source, p), []byte(body), 0600); err != nil {
			t.Fatal(err)
		}
	}
	if b, err := exec.Command("git", "-C", c.Source, "init", "-q").CombinedOutput(); err != nil {
		t.Fatalf("%v %s", err, b)
	}
	policy, err := native.New(c)
	if err != nil {
		t.Fatal(err)
	}
	if err = policy.SaveIdentity(native.Machine{ID: "linux_ui", OS: "linux"}); err != nil {
		t.Fatal(err)
	}
	return NewNativeEngine(policy), c
}
func TestNativeAdapterUsesRealReviewedApply(t *testing.T) {
	e, c := realNative(t)
	ctx := context.Background()
	inv, err := e.InventoryMode(ctx, "adopt")
	if err != nil || len(inv.Entries) != 1 {
		t.Fatalf("%+v %v", inv, err)
	}
	p, err := e.Plan(ctx, map[string]any{"version": 1, "mode": "adopt", "profile": "base", "entries": []string{"prefs"}})
	if err != nil {
		t.Fatal(err)
	}
	preview, err := e.Preview(ctx, p.Plan)
	if err != nil || !preview.GatesKnown || len(preview.Diffs) != 1 {
		t.Fatalf("%+v %v", preview, err)
	}
	if err = e.Apply(ctx, p.Plan, p.Plan["id"].(string), false, false, false); err != nil {
		t.Fatal(err)
	}
	b, _ := os.ReadFile(filepath.Join(c.Home, "prefs"))
	if string(b) != "real bytes\n" {
		t.Fatal("adapter did not apply")
	}
}

func TestReturningInspectionUsesApplicationContext(t *testing.T) {
	e, _ := realNative(t)
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	m := NewReturningModelContext(ctx, e)
	for i := 0; i < 3; i++ {
		m, _ = m.Update(tea.KeyPressMsg{Code: 'j'})
	}
	m, cmd := m.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	if cmd == nil {
		t.Fatal("missing inspection command")
	}
	m, _ = m.Update(cmd())
	if m.(model).err == nil {
		t.Fatal("inspection ignored canceled application context")
	}
}

func TestLogicalEditorDefaultPlansAgainstNativeSchema(t *testing.T) {
	e, c := realNative(t)
	if err := os.WriteFile(filepath.Join(c.Source, "dot-contract.json"), []byte(`{"protocol":1,"engine_api":{"min":1,"max":1},"manifest_version":2,"approval_version":3,"manifest":"manifest.json","home_source":"home","required_adapters":["intent","scope"]}`), 0600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(c.Source, "manifest.json"), []byte(`{"version":2,"entries":{},"profiles":{},"tracking":{}}`), 0600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(c.Home, "prefs"), []byte("logical bytes\n"), 0600); err != nil {
		t.Fatal(err)
	}
	m := NewModel(e, false, "track")
	m, cmd := m.Update(m.Init()())
	for cmd != nil {
		m, cmd = m.Update(cmd())
	}
	m = send(m, "M")
	m, _ = m.Update(tea.KeyPressMsg{Code: 19})
	m = sendAndRun(m, "p")
	if m.(model).stage != "review" || m.(model).err != nil {
		t.Fatalf("native logical plan: %s %v", m.(model).stage, m.(model).err)
	}
}
func TestReturningMenuDefaultsFinishAndRemoteDefaultsCancel(t *testing.T) {
	e, c := realNative(t)
	m := NewReturningModel(e).(model)
	if m.stage != "returning-home" || m.returningChoice != 0 {
		t.Fatal("Finish not default")
	}
	m.returningChoice = 6
	next, cmd := m.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	m = next.(model)
	if cmd != nil || m.stage != "returning-remote-confirm" || m.confirmChoice != 0 {
		t.Fatal("pull performed without separate cancel-default confirmation")
	}
	next, cmd = m.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	m = next.(model)
	if cmd != nil || m.stage != "returning-home" {
		t.Fatal("Cancel executed remote")
	}
	if _, err := os.Stat(filepath.Join(c.Home, "prefs")); !os.IsNotExist(err) {
		t.Fatal("navigation adopted")
	}
	m.returningChoice = 3
	next, cmd = m.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	if cmd == nil {
		t.Fatal("status unavailable")
	}
	next, _ = next.Update(cmd())
	m = next.(model)
	if m.stage != "returning-inspect" || m.err != nil {
		t.Fatalf("status %s %v", m.stage, m.err)
	}
}
func TestRealBrowserRetargetReviewsAndCancelWritesNothing(t *testing.T) {
	e, c := realNative(t)
	if err := os.WriteFile(filepath.Join(c.Home, "prefs"), []byte("local bytes\n"), 0600); err != nil {
		t.Fatal(err)
	}
	before, _ := os.ReadFile(filepath.Join(c.Source, "manifest.json"))
	m := NewModel(e, false, "track").(model)
	next, cmd := m.Update(m.Init()())
	m = next.(model)
	if cmd != nil {
		next, _ = m.Update(cmd())
		m = next.(model)
	}
	next, _ = m.Update(tea.KeyPressMsg{Code: tea.KeyTab})
	m = next.(model)
	next, cmd = m.Update(tea.KeyPressMsg{Code: 'p'})
	if cmd == nil {
		t.Fatal("retarget did not prepare exact selection")
	}
	next, cmd = next.Update(cmd())
	if cmd == nil {
		t.Fatal("native plan was rejected before preview")
	}
	next, _ = next.Update(cmd())
	m = next.(model)
	if m.stage != "review" || m.selected == nil || !m.selected.GatesKnown {
		t.Fatalf("review unavailable: %s %v", m.stage, m.err)
	}
	next, _ = m.Update(tea.KeyPressMsg{Code: 'a'})
	m = next.(model)
	if m.stage != "confirm" || m.confirmChoice != 0 {
		t.Fatal("final confirmation not cancel-default")
	}
	next, cmd = m.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	if cmd != nil {
		t.Fatal("Cancel executed native apply")
	}
	m = next.(model)
	after, _ := os.ReadFile(filepath.Join(c.Source, "manifest.json"))
	if string(before) != string(after) {
		t.Fatal("retarget/navigation/cancel mutated source")
	}
	next, _ = m.Update(tea.KeyPressMsg{Code: 'a'})
	next, _ = next.Update(tea.KeyPressMsg{Code: tea.KeyDown})
	next, cmd = next.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	if cmd == nil {
		t.Fatal("explicit reviewed Track did not apply")
	}
	next, _ = next.Update(cmd())
	m = next.(model)
	if m.err != nil {
		t.Fatal(m.err)
	}
	after, _ = os.ReadFile(filepath.Join(c.Source, "manifest.json"))
	if string(before) == string(after) {
		t.Fatal("real Track adapter did not persist retarget")
	}
	a, err := e.Policy.LoadAdoption()
	if err != nil || len(a.Entries) != 0 {
		t.Fatal("Track implicitly adopted")
	}
}

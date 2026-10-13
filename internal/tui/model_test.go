package tui

import (
	"context"
	"fmt"
	"strings"
	"testing"

	tea "charm.land/bubbletea/v2"
)

type fixtureEngine struct {
	inventory       Inventory
	plans           int
	previews        int
	applies         int
	request         map[string]any
	unsafe          bool
	desktop         bool
	backup          bool
	acks            []bool
	highlightResult HighlightResult
}

func (f *fixtureEngine) InventoryMode(context.Context, string) (Inventory, error) {
	return f.inventory, nil
}
func (f *fixtureEngine) Snapshot(_ context.Context, path string) (SnapshotResult, error) {
	prefix := path + "/"
	var paths []string
	for _, e := range f.inventory.Entries {
		if e.Kind == "file" && e.Status == "eligible" && strings.HasPrefix(e.Path, prefix) {
			paths = append(paths, e.Path)
		}
	}
	return SnapshotResult{Paths: paths}, nil
}
func (f *fixtureEngine) Highlight(context.Context, string, string, map[string]any) (HighlightResult, error) {
	if f.highlightResult.Status != "" {
		return f.highlightResult, nil
	}
	return HighlightResult{Status: "safe", Direction: "HOME → repository", Summary: "Fixture preview", Text: "safe preview body"}, nil
}
func (f *fixtureEngine) Plan(_ context.Context, r map[string]any) (PlanResult, error) {
	f.plans++
	f.request = r
	return PlanResult{Plan: map[string]any{"id": "plan-fixture"}, Direction: "HOME → repository", Diffs: []Diff{{Path: ".config/prefs", Status: "safe", Text: "-old\n+new"}}, GatesKnown: true, Unsafe: f.unsafe, Desktop: f.desktop, Backup: f.backup}, nil
}
func (f *fixtureEngine) Preview(_ context.Context, plan any) (PlanResult, error) {
	f.previews++
	return PlanResult{Plan: plan.(map[string]any), Direction: "HOME → repository", Diffs: []Diff{{Path: ".config/prefs", Status: "safe", Text: "-old\n+new"}}, GatesKnown: true}, nil
}
func (f *fixtureEngine) Apply(_ context.Context, _ any, _ string, unsafe, desktop, backup bool) error {
	f.applies++
	f.acks = []bool{unsafe, desktop, backup}
	return nil
}

func key(s string) tea.KeyPressMsg {
	code := tea.Key{Code: []rune(s)[0]}
	switch s {
	case "enter":
		code = tea.Key{Code: tea.KeyEnter}
	case "esc":
		code = tea.Key{Code: tea.KeyEsc}
	case " ":
		code = tea.Key{Code: tea.KeySpace}
	}
	return tea.KeyPressMsg(code)
}
func send(m tea.Model, s string) tea.Model { next, _ := m.Update(key(s)); return next }
func sendAndRun(m tea.Model, s string) tea.Model {
	next, cmd := m.Update(key(s))
	for i := 0; cmd != nil && i < 3; i++ {
		next, cmd = next.Update(cmd())
	}
	return next
}
func boot(t *testing.T, engine *fixtureEngine, rounds bool, mode string) tea.Model {
	t.Helper()
	m := NewModel(engine, rounds, mode)
	if cmd := m.Init(); cmd != nil {
		msg := cmd()
		m, _ = m.Update(msg)
	}
	return m
}

func TestPublicTerminalBasketSnapshotsOnlyCurrentEligibleFiles(t *testing.T) {
	e := &fixtureEngine{inventory: Inventory{Entries: []Entry{{Path: "config", Kind: "directory", Status: "eligible", Open: true}, {Path: "config/a", Kind: "file", Status: "eligible"}, {Path: "config/auth.json", Kind: "file", Status: "excluded"}}}}
	m := boot(t, e, false, "track")
	m = sendAndRun(m, " ")
	view := m.View().Content
	if !strings.Contains(view, "1 selected") || !strings.Contains(view, "Snapshotted 1 eligible exact files") {
		t.Fatalf("folder snapshot not visible to user: %s", view)
	}
	e.inventory.Entries = append(e.inventory.Entries, Entry{Path: ".config/later", Kind: "file", Status: "eligible"})
	view = m.View().Content
	if !strings.Contains(view, "1 selected") {
		t.Fatalf("later inventory change expanded basket: %s", view)
	}
}

func TestPublicTerminalSearchAndHiddenToggleDoNotDropBasket(t *testing.T) {
	e := &fixtureEngine{inventory: Inventory{Entries: []Entry{{Path: "plain", Kind: "file", Status: "eligible"}, {Path: ".private", Kind: "file", Status: "eligible", Metadata: map[string]any{"destination": ".private", "os": []string{"darwin"}, "portability": "managed-shared", "impact": "Safe"}}}}}
	m := boot(t, e, false, "track")
	view := m.View().Content
	if strings.Contains(view, ".private") {
		t.Fatalf("hidden entry shown before toggle: %s", view)
	}
	m = send(m, "H")
	if !strings.Contains(m.View().Content, ".private") {
		t.Fatal("H did not reveal hidden entry")
	}
	m = sendAndRun(m, " ")
	m = send(m, "/")
	for _, r := range "plain" {
		m, _ = m.Update(tea.KeyPressMsg(tea.Key{Code: r}))
	}
	if !strings.Contains(m.View().Content, "1 selected") {
		t.Fatalf("search editor hid the exact basket count: %s", m.View().Content)
	}
	m = send(m, "enter")
	if !strings.Contains(m.View().Content, "1 selected") {
		t.Fatalf("search lost exact basket selection: %s", m.View().Content)
	}
}

func TestPublicPreviewUsesGenerationAndRendersDirectionalSanitizedDiff(t *testing.T) {
	e := &fixtureEngine{inventory: Inventory{Entries: []Entry{{Path: "prefs", Kind: "file", Status: "eligible", Metadata: map[string]any{"destination": "prefs", "os": []string{"darwin"}, "portability": "managed-shared", "impact": "Safe"}}}}}
	m := boot(t, e, false, "track")
	m = send(m, " ")
	m = sendAndRun(m, "p")
	if m.View().Content == "" {
		t.Fatal("terminal view vanished after plan and preview requests")
	}
	if e.plans != 1 || e.previews != 1 {
		t.Fatalf("plan/preview were not called: %#v", e)
	}
	if got := m.View().Content; !strings.Contains(got, "Plan ID: plan-fixture") || !strings.Contains(got, "-old") {
		t.Fatalf("review does not show exact plan and diff: %s", got)
	}
}

func TestPublicOptionalRoundsFinishWithoutCallingEngine(t *testing.T) {
	e := &fixtureEngine{}
	m := NewModel(e, true, "track")
	if cmd := m.Init(); cmd != nil {
		t.Fatal("round menu should not inventory before choice")
	}
	if !strings.Contains(m.View().Content, "Finish without further adoption") {
		t.Fatal("rounds do not show Finish option")
	}
	_, cmd := m.Update(key("enter"))
	if cmd == nil {
		t.Fatal("Finish did not exit optional rounds")
	}
	if e.plans != 0 || e.applies != 0 {
		t.Fatal("Finish invoked plan/apply")
	}
}

func TestPublicAdoptUsesManifestIDsAndEngineOSScope(t *testing.T) {
	e := &fixtureEngine{inventory: Inventory{Entries: []Entry{
		{Path: ".config", Kind: "directory", Status: "logical"},
		{Path: ".config/prefs", ID: "manifest-prefs", Kind: "file", Status: "managed", OSStatus: "supported", Approved: true},
		{Path: ".config/linux-only", ID: "linux-only", Kind: "file", Status: "managed", OSStatus: "unsupported"},
	}}}
	m := boot(t, e, false, "adopt")
	m = send(m, "H")
	m = send(m, "l")
	m = send(m, "j")
	m = send(m, "j")
	if !strings.Contains(m.View().Content, "Scope: true") {
		t.Fatalf("approved-scope details missing: %s", m.View().Content)
	}
	m = send(m, " ")
	m = sendAndRun(m, "p")
	if !strings.Contains(m.View().Content, "REPOSITORY") || e.request == nil {
		t.Fatalf("adopt details/plan missing: %s", m.View().Content)
	}
	entries, ok := e.request["entries"].([]string)
	if !ok || len(entries) != 1 || entries[0] != "manifest-prefs" {
		t.Fatalf("Adopt did not submit exact manifest ID: %#v", e.request)
	}
}

func TestPublicSeparateConsentGatesAndCancelDefaultNeverApply(t *testing.T) {
	e := &fixtureEngine{unsafe: true, desktop: true, backup: true, inventory: Inventory{Entries: []Entry{{Path: "prefs", Kind: "file", Status: "eligible", Metadata: map[string]any{"destination": "prefs", "os": []string{"darwin"}, "portability": "managed-shared", "impact": "Unsafe"}}}}}
	m := boot(t, e, false, "track")
	m = send(m, " ")
	m = sendAndRun(m, "p")
	m = send(m, "a")
	if !strings.Contains(m.View().Content, "Unsafe acknowledgment required separately") || e.applies != 0 {
		t.Fatalf("Unsafe gate did not stop review: %s", m.View().Content)
	}
	m = send(m, "u")
	m = send(m, "a")
	if !strings.Contains(m.View().Content, "Desktop acknowledgment required separately") || e.applies != 0 {
		t.Fatalf("Desktop gate was not separate: %s", m.View().Content)
	}
	m = send(m, "d")
	m = send(m, "a")
	if !strings.Contains(m.View().Content, "Backup replacement acknowledgment required separately") || e.applies != 0 {
		t.Fatalf("backup gate was not separate: %s", m.View().Content)
	}
	m = send(m, "b")
	m = send(m, "a")
	if !strings.Contains(m.View().Content, "> Cancel") {
		t.Fatalf("final confirmation did not default to Cancel: %s", m.View().Content)
	}
	m = send(m, "enter")
	if e.applies != 0 {
		t.Fatal("default Cancel applied the plan")
	}
	m = send(m, "u")
	m = send(m, "d")
	m = send(m, "b")
	m = send(m, "a")
	m = send(m, "j")
	m = sendAndRun(m, "enter")
	if e.applies != 1 || len(e.acks) != 3 || !e.acks[0] || !e.acks[1] || !e.acks[2] {
		t.Fatalf("separate engine flags not forwarded: %#v; view=%s", e.acks, m.View().Content)
	}
}

func TestPublicStaleAsyncPlanResultIsDiscardedAfterSelectionNavigation(t *testing.T) {
	e := &fixtureEngine{inventory: Inventory{Entries: []Entry{{Path: "alpha", Kind: "file", Status: "eligible", Metadata: map[string]any{"destination": "alpha", "os": []string{"darwin"}, "portability": "managed-shared", "impact": "Safe"}}, {Path: "beta", Kind: "file", Status: "eligible", Metadata: map[string]any{"destination": "beta", "os": []string{"darwin"}, "portability": "managed-shared", "impact": "Safe"}}}}}
	m := boot(t, e, false, "track")
	m = send(m, " ")
	m, oldCmd := m.Update(key("p"))
	m, newCmd := m.Update(key("p"))
	if oldCmd == nil || newCmd == nil {
		t.Fatal("preview plan commands were not issued")
	}
	m, _ = m.Update(oldCmd())
	if strings.Contains(m.View().Content, "REVIEW EXACT PLAN") {
		t.Fatal("stale plan result reached review")
	}
	m = runCmds(m, newCmd)
	if !strings.Contains(m.View().Content, "REVIEW EXACT PLAN") {
		t.Fatalf("current plan did not reach review: %s", m.View().Content)
	}
}

func TestPublicNarrowTerminalPreviewScrollsWithoutHidingDirection(t *testing.T) {
	lines := make([]string, 40)
	for i := range lines {
		lines[i] = fmt.Sprintf("line-%02d", i)
	}
	e := &fixtureEngine{inventory: Inventory{Entries: []Entry{{Path: "prefs", Kind: "file", Status: "eligible", Metadata: map[string]any{"destination": "prefs", "os": []string{"darwin"}, "portability": "managed-shared", "impact": "Safe"}}}}, highlightResult: HighlightResult{Status: "safe", Direction: "HOME → repository", Summary: "long fixture", Text: strings.Join(lines, "\n")}}
	m := boot(t, e, false, "track")
	m, _ = m.Update(tea.WindowSizeMsg{Width: 70, Height: 20})
	m = sendAndRun(m, "j")
	view := m.View().Content
	if !strings.Contains(view, "HOME → REPOSITORY") || !strings.Contains(view, "line-00") {
		t.Fatalf("narrow preview is missing direction/content: %s", view)
	}
	for i := 0; i < 22; i++ {
		m = send(m, "J")
	}
	if !strings.Contains(m.View().Content, "line-20") {
		t.Fatalf("preview did not scroll in narrow layout: %s", m.View().Content)
	}
}

func runCmds(m tea.Model, cmd tea.Cmd) tea.Model {
	for i := 0; cmd != nil && i < 3; i++ {
		m, cmd = m.Update(cmd())
	}
	return m
}

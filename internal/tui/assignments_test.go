package tui

import (
	"reflect"
	"strings"
	"testing"

	tea "charm.land/bubbletea/v2"
)

func TestPublicTabRetargetsExactTrackEntryWithoutApplying(t *testing.T) {
	e := &fixtureEngine{inventory: Inventory{Machine: Machine{ID: "local_machine", OS: "darwin"}, Machines: []string{"local_machine", "other_machine"}, Entries: []Entry{{Path: "prefs", Kind: "file", Status: "eligible", Metadata: map[string]any{"destination": "prefs", "os": []string{"darwin"}, "portability": "managed-shared", "impact": "Safe"}}}}}
	m := boot(t, e, false, "track")
	m = send(m, " ")
	tab := tea.KeyPressMsg(tea.Key{Code: tea.KeyTab})
	// Retargeting cycles All machines ↔ this machine only; broader scopes
	// never cycle here and labels stay human, never raw JSON/OS names.
	for _, step := range []struct {
		label string
		want  string
	}{{"local_machine only", "local_machine"}, {"All machines", "all"}, {"local_machine only", "local_machine"}} {
		m, _ = m.Update(tab)
		if !strings.Contains(m.View().Content, "Assignment: "+step.label) {
			t.Fatalf("assignment is not visible: %s", m.View().Content)
		}
		m = sendAndRun(m, "p")
		selections := e.request["selections"].([]map[string]any)
		if !reflect.DeepEqual(selections[0]["machines"], map[string]any{"include": []string{step.want}, "exclude": []string{}}) {
			t.Fatalf("wrong exact-plan assignment: %#v", e.request)
		}
		m = send(m, "esc")
	}
	if e.applies != 0 {
		t.Fatal("Tab or review authorized apply")
	}
}

func TestPublicAdoptPresetsComposeAndDeselectLocally(t *testing.T) {
	e := &fixtureEngine{inventory: Inventory{Profiles: []Profile{{Name: "base", Entries: []string{"prefs"}}, {Name: "extra", Entries: []string{"prefs", "extra"}}}, Entries: []Entry{{Path: "extra", ID: "extra", Kind: "file", Status: "managed", OSStatus: "supported"}, {Path: "prefs", ID: "prefs", Kind: "file", Status: "managed", OSStatus: "supported"}}}}
	m := boot(t, e, false, "adopt")
	m = send(m, "s")
	m = send(m, "enter")
	m = send(m, "s")
	m = send(m, "j")
	m = send(m, "enter")
	if !strings.Contains(m.View().Content, "2 selected") {
		t.Fatalf("union not visible: %s", m.View().Content)
	}
	m = send(m, " ") // deselect extra; never edits shared profiles
	m = sendAndRun(m, "p")
	if !reflect.DeepEqual(e.request["profiles"], []string{"base", "extra"}) || !reflect.DeepEqual(e.request["entries"], []string{"prefs"}) {
		t.Fatalf("composed profiles/local deselection not sent: %#v", e.request)
	}
	if e.applies != 0 {
		t.Fatal("preset selection applied without approval")
	}
}

func TestPublicAdoptCanReviewDeselectingEveryFile(t *testing.T) {
	e := &fixtureEngine{inventory: Inventory{Profiles: []Profile{{Name: "base", Entries: []string{"prefs"}}}, Entries: []Entry{{Path: "prefs", ID: "prefs", Kind: "file", Status: "managed", OSStatus: "supported"}}}}
	m := boot(t, e, false, "adopt")
	m = send(m, "s")
	m = send(m, "enter")
	m = send(m, " ")
	m = sendAndRun(m, "p")
	if e.plans != 1 || !reflect.DeepEqual(e.request["entries"], []string{}) {
		t.Fatalf("cannot review explicit empty local view: %#v", e.request)
	}
	if e.applies != 0 {
		t.Fatal("deselection applied without exact approval")
	}
}

func TestPublicTabRetargetsLogicalFileThroughTrack(t *testing.T) {
	e := &fixtureEngine{inventory: Inventory{Machine: Machine{ID: "local_machine", OS: "darwin"}, Machines: []string{"local_machine", "other_machine"}, Entries: []Entry{{Path: "prefs", ID: "logical@darwin", LogicalID: "logical", Kind: "file", Status: "managed", Metadata: map[string]any{"machines": map[string]any{"include": []string{"all"}, "exclude": []string{}}}, Tracking: map[string]any{"destinations": map[string]string{"darwin": "prefs"}, "sources": map[string]string{"darwin": "home/prefs"}, "common": map[string]any{}, "machine": map[string]any{}, "os_metadata": map[string]any{}, "machines": map[string]any{"include": []string{"all"}, "exclude": []string{}}}}}}}
	m := boot(t, e, false, "track")
	m, _ = m.Update(tea.KeyPressMsg(tea.Key{Code: tea.KeyTab}))
	if !strings.Contains(m.(model).message, "Assignment: local_machine only") {
		t.Fatalf("retarget did not use the human machine-only label: %q", m.(model).message)
	}
	m = sendAndRun(m, "p")
	if e.plans != 1 {
		t.Fatal("logical Tab did not prepare a Track plan")
	}
	selection := e.request["selections"].([]map[string]any)[0]
	if selection["logical"] != "logical" || selection["destinations"] == nil || selection["sources"] != nil {
		t.Fatalf("incorrect logical Track request: %#v", selection)
	}
	if !reflect.DeepEqual(selection["machines"], map[string]any{"include": []string{"local_machine"}, "exclude": []string{}}) {
		t.Fatalf("logical assignment was lost: %#v", selection)
	}
}

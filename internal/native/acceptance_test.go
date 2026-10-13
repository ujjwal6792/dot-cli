package native_test

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"testing"

	"github.com/ujjwal6792/dots-cli/internal/native"
)

func TestEntryAssignmentRejectsLegacyList(t *testing.T) {
	for _, value := range []string{`[]`, `["all"]`} {
		var r native.Request
		if err := native.Decode([]byte(`{"version":1,"mode":"track","profile":"base","entries":[],"selections":[{"path":"prefs","machines":`+value+`}]}`), &r); err == nil {
			t.Fatalf("accepted new entry assignment %s", value)
		}
	}
}

func TestLegacyLogicalAssignmentKeepsStrictJSON(t *testing.T) {
	var logical native.Logical
	if err := json.Unmarshal([]byte(`{"machines":[],"common":{"impact":"Unsafe"},"common":{"impact":"Safe"}}`), &logical); err == nil {
		t.Fatal("legacy assignment normalization discarded a duplicate capability")
	}
}

func TestLocalDeselectionSurvivesProfileSwitch(t *testing.T) {
	e, c := fixture(t)
	ctx := context.Background()
	var m native.Manifest
	b, _ := os.ReadFile(filepath.Join(c.Source, "manifest.json"))
	if err := native.Decode(b, &m); err != nil {
		t.Fatal(err)
	}
	m.Profiles["empty"] = []string{}
	writeJSON(t, filepath.Join(c.Source, "manifest.json"), m)
	for _, r := range []native.Request{
		{Version: 1, Mode: "adopt", Profile: "base", Entries: []string{}},
		{Version: 1, Mode: "adopt", Profile: "empty", Entries: []string{}},
	} {
		p, err := e.Plan(ctx, r)
		if err != nil {
			t.Fatal(err)
		}
		if err := e.Apply(ctx, p, native.Consent{Approve: p.ID}); err != nil {
			t.Fatal(err)
		}
	}
	p, err := e.Plan(ctx, native.Request{Version: 1, Mode: "adopt", Profile: "base"})
	if err != nil {
		t.Fatal(err)
	}
	if len(p.Selected) != 0 {
		t.Fatal("profile switching forgot machine-local exclusion")
	}
}

func TestSystemGitConfigurationApplies(t *testing.T) {
	e, c := fixture(t)
	config := filepath.Join(c.Home, "system-gitconfig")
	write(t, config, "[safe]\n directory = /never-the-fixture\n", 0600)
	t.Setenv("GIT_CONFIG_SYSTEM", config)
	t.Setenv("GIT_CONFIG_NOSYSTEM", "")
	t.Setenv("GIT_TEST_ASSUME_DIFFERENT_OWNER", "1")
	if _, err := e.CheckContract(context.Background()); err == nil {
		t.Fatal("Git system safe.directory policy was disabled")
	}
	git(t, c.Source, "config", "--file", config, "--add", "safe.directory", c.Source)
	if _, err := e.CheckContract(context.Background()); err != nil {
		t.Fatal(err)
	}
}

func TestLogicalTrackPlanSurvivesSerialization(t *testing.T) {
	e, c := fixture(t)
	writeJSON(t, filepath.Join(c.Source, "dot-contract.json"), native.Contract{Protocol: 1, EngineAPI: native.VersionRange{Min: 1, Max: 1}, ManifestVersion: 2, ApprovalVersion: 3, Manifest: "manifest.json", HomeSource: "home", RequiredAdapters: []string{"intent", "scope"}})
	writeJSON(t, filepath.Join(c.Source, "manifest.json"), native.Manifest{Version: 2, Entries: map[string]native.Entry{}, Profiles: map[string][]string{}})
	write(t, filepath.Join(c.Home, "prefs"), "logical bytes\n", 0600)
	p, err := e.Plan(context.Background(), native.Request{Version: 1, Mode: "track", Profile: "base", Selections: []native.Selection{{Path: "prefs", Logical: "prefs", Destinations: map[string]string{"linux": "prefs"}, Common: map[string]any{"impact": "Safe", "portability": "managed-shared"}, Machine: map[string]map[string]any{}, OSMetadata: map[string]map[string]any{}}}})
	if err != nil {
		t.Fatal(err)
	}
	b, err := json.Marshal(p)
	if err != nil {
		t.Fatal(err)
	}
	var decoded native.Plan
	if err = native.Decode(b, &decoded); err != nil {
		t.Fatal(err)
	}
	if _, err = e.Preview(context.Background(), decoded); err != nil {
		t.Fatal(err)
	}
}

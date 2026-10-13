package native_test

import (
	"context"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"

	"github.com/ujjwal6792/dots-cli/internal/native"
)

func fixture(t *testing.T) (*native.Engine, native.Config) {
	t.Helper()
	root, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	c := native.Config{Source: filepath.Join(root, "source"), Home: filepath.Join(root, "home"), State: filepath.Join(root, "state"), OS: "linux", Git: "git"}
	for _, p := range []string{c.Source, c.Home, c.State, filepath.Join(c.Source, "home")} {
		if err := os.MkdirAll(p, 0700); err != nil {
			t.Fatal(err)
		}
	}
	writeJSON(t, filepath.Join(c.Source, "dot-contract.json"), native.Contract{Protocol: 1, EngineAPI: native.VersionRange{Min: 1, Max: 1}, ManifestVersion: 1, ApprovalVersion: 3, Manifest: "manifest.json", HomeSource: "home", RequiredAdapters: []string{"intent", "scope", "gitconfig", "skills", "layout"}})
	writeJSON(t, filepath.Join(c.Source, "manifest.json"), native.Manifest{Version: 1, Entries: map[string]native.Entry{"prefs": {Source: "home/prefs", Destination: "prefs", OS: []string{"linux"}, Portability: "managed-shared", Impact: "Safe"}}, Profiles: map[string][]string{"base": {"prefs"}}})
	write(t, filepath.Join(c.Source, "home/prefs"), "shared\n", 0751)
	writeJSON(t, filepath.Join(c.State, "adoption.json"), native.Adoption{Version: 3, Profile: "unapproved", Entries: map[string]native.Entry{}, Machine: native.Machine{ID: "linux_test", OS: "linux"}})
	git(t, c.Source, "init", "-q")
	e, err := native.New(c)
	if err != nil {
		t.Fatal(err)
	}
	return e, c
}
func write(t *testing.T, p, s string, m os.FileMode) {
	t.Helper()
	if err := os.WriteFile(p, []byte(s), m); err != nil {
		t.Fatal(err)
	}
	if err := os.Chmod(p, m); err != nil {
		t.Fatal(err)
	}
}
func writeJSON(t *testing.T, p string, v any) {
	t.Helper()
	b, err := json.Marshal(v)
	if err != nil {
		t.Fatal(err)
	}
	write(t, p, string(b), 0600)
}
func git(t *testing.T, p string, args ...string) {
	t.Helper()
	cmd := exec.Command("git", append([]string{"-C", p}, args...)...)
	if b, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("git %v: %s %v", args, b, err)
	}
}
func TestExactAdoptionPreservesBytesModeAndMachine(t *testing.T) {
	e, c := fixture(t)
	ctx := context.Background()
	p, err := e.Plan(ctx, native.Request{Version: 1, Mode: "adopt", Profile: "base", Entries: []string{"prefs"}})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(filepath.Join(c.Home, "prefs")); !os.IsNotExist(err) {
		t.Fatal("planning wrote target")
	}
	if err := e.Apply(ctx, p, native.Consent{Approve: "wrong"}); err == nil {
		t.Fatal("accepted wrong approval")
	}
	if err := e.Apply(ctx, p, native.Consent{Approve: p.ID}); err != nil {
		t.Fatal(err)
	}
	b, _ := os.ReadFile(filepath.Join(c.Home, "prefs"))
	info, _ := os.Stat(filepath.Join(c.Home, "prefs"))
	if string(b) != "shared\n" || info.Mode().Perm() != 0751 {
		t.Fatalf("bytes/mode %q %v", b, info.Mode())
	}
	s, err := e.LoadAdoption()
	if err != nil || s.Machine.ID != "linux_test" || len(s.Entries) != 1 {
		t.Fatalf("state %+v %v", s, err)
	}
}
func TestTrackSnapshotsExactFilesWithoutAdopting(t *testing.T) {
	e, c := fixture(t)
	write(t, filepath.Join(c.Home, "custom"), "local\n", 0701)
	before, _ := os.ReadFile(filepath.Join(c.State, "adoption.json"))
	p, err := e.Plan(context.Background(), native.Request{Version: 1, Mode: "track", Profile: "mine", Selections: []native.Selection{{Path: "custom", Entry: native.Entry{OS: []string{"linux"}, Portability: "managed-shared", Impact: "Safe"}}}})
	if err != nil {
		t.Fatal(err)
	}
	if p.Selected["custom"].Machines == nil || p.Selected["custom"].Machines.Include[0] != "all" {
		t.Fatal("missing default assignment")
	}
	if err = e.Apply(context.Background(), p, native.Consent{Approve: p.ID}); err != nil {
		t.Fatal(err)
	}
	b, _ := os.ReadFile(filepath.Join(c.Source, "home/custom"))
	after, _ := os.ReadFile(filepath.Join(c.State, "adoption.json"))
	if string(b) != "local\n" || string(before) != string(after) {
		t.Fatal("Track failed or adopted")
	}
	write(t, filepath.Join(c.Home, "future"), "not selected", 0600)
	var m native.Manifest
	b, _ = os.ReadFile(filepath.Join(c.Source, "manifest.json"))
	if err = native.Decode(b, &m); err != nil {
		t.Fatal(err)
	}
	if len(m.Entries) != 2 {
		t.Fatal("scope broadened")
	}
}
func TestBoundedGitAdoptionPreservesUnapprovedSections(t *testing.T) {
	e, c := fixture(t)
	var m native.Manifest
	b, _ := os.ReadFile(filepath.Join(c.Source, "manifest.json"))
	_ = native.Decode(b, &m)
	m.Entries["git"] = native.Entry{Source: "home/git", Destination: ".gitconfig", OS: []string{"linux"}, Portability: "managed-templated", Impact: "Unsafe", Integrations: []string{"git-config"}, GitKeys: []string{"user.name"}, GitValues: map[string]string{"user.name": "Reviewed"}}
	m.Profiles["git"] = []string{"git"}
	writeJSON(t, filepath.Join(c.Source, "manifest.json"), m)
	live := "[user]\n\tname = Old\n\temail = private@example.invalid\n[credential]\n\thelper = unchanged\n# preserve comment\n"
	write(t, filepath.Join(c.Home, ".gitconfig"), live, 0640)
	p, err := e.Plan(context.Background(), native.Request{Version: 1, Mode: "adopt", Profile: "git"})
	if err != nil {
		t.Fatal(err)
	}
	if err = e.Apply(context.Background(), p, native.Consent{Approve: p.ID}); err == nil {
		t.Fatal("Unsafe not gated")
	}
	if err = e.Apply(context.Background(), p, native.Consent{Approve: p.ID, Unsafe: true}); err != nil {
		t.Fatal(err)
	}
	b, _ = os.ReadFile(filepath.Join(c.Home, ".gitconfig"))
	if string(b) != "[user]\n\tname = Reviewed\n\temail = private@example.invalid\n[credential]\n\thelper = unchanged\n# preserve comment\n" {
		t.Fatalf("unbounded Git change: %s", b)
	}
}
func TestMachineScopedOfflineInspectionAndProfileUnion(t *testing.T) {
	e, c := fixture(t)
	var m native.Manifest
	b, _ := os.ReadFile(filepath.Join(c.Source, "manifest.json"))
	_ = native.Decode(b, &m)
	other := m.Entries["prefs"]
	other.Source = "home/other"
	other.Destination = "other"
	other.Machines = &native.Assignment{Include: []string{"all"}, Exclude: []string{"linux_test"}}
	m.Entries["other"] = other
	m.Profiles["extra"] = []string{"prefs", "other"}
	writeJSON(t, filepath.Join(c.Source, "manifest.json"), m)
	write(t, filepath.Join(c.Source, "home/other"), "other\n", 0600)
	inv, err := e.InventoryMode(context.Background(), "adopt")
	if err != nil {
		t.Fatal(err)
	}
	if len(inv.Entries) != 2 {
		t.Fatalf("inventory %+v", inv)
	}
	for _, x := range inv.Entries {
		if x.ID == "other" && x.OSStatus != "unsupported" {
			t.Fatal("machine exclusion ignored")
		}
	}
	diffs, err := e.Diff(context.Background())
	if err != nil || len(diffs) != 1 {
		t.Fatalf("machine diff %+v %v", diffs, err)
	}
	p, err := e.Plan(context.Background(), native.Request{Version: 1, Mode: "adopt", Profile: "selection", Profiles: []string{"base", "extra"}})
	if err != nil || len(p.Selected) != 1 {
		t.Fatalf("union %+v %v", p.Selected, err)
	}
	if err = e.Apply(context.Background(), p, native.Consent{Approve: p.ID}); err != nil {
		t.Fatal(err)
	}
	s, err := e.ScopeShow(context.Background())
	if err != nil || len(s.Entries) != 1 || s.Machine.ID != "linux_test" {
		t.Fatalf("scope %+v %v", s, err)
	}
	encoded, _ := json.Marshal(s)
	if !strings.Contains(string(encoded), `"machine":{"id":"linux_test","os":"linux"}`) {
		t.Fatal(string(encoded))
	}
}
func TestAdoptionLocalDeselectionsCustomsAndUpdateStayExact(t *testing.T) {
	e, c := fixture(t)
	var m native.Manifest
	b, _ := os.ReadFile(filepath.Join(c.Source, "manifest.json"))
	_ = native.Decode(b, &m)
	extra := m.Entries["prefs"]
	extra.Source = "home/extra"
	extra.Destination = "extra"
	m.Entries["extra"] = extra
	m.Profiles["base"] = []string{"prefs", "extra"}
	writeJSON(t, filepath.Join(c.Source, "manifest.json"), m)
	write(t, filepath.Join(c.Source, "home/extra"), "extra", 0600)
	before, _ := os.ReadFile(filepath.Join(c.Source, "manifest.json"))
	req := native.Request{Version: 1, Mode: "adopt", Profile: "base", Entries: []string{"prefs"}, Metadata: map[string]map[string]any{"prefs": {"tools": []string{"local-editor"}}}}
	p, err := e.Plan(context.Background(), req)
	if err != nil {
		t.Fatal(err)
	}
	if err = e.Apply(context.Background(), p, native.Consent{Approve: p.ID}); err != nil {
		t.Fatal(err)
	}
	p, err = e.Plan(context.Background(), native.Request{Version: 1, Mode: "update", Profile: "base"})
	if err != nil || len(p.Selected) != 1 || p.Selected["prefs"].Tools[0] != "local-editor" {
		t.Fatalf("update %+v %v", p, err)
	}
	if err = e.Apply(context.Background(), p, native.Consent{Approve: p.ID}); err != nil {
		t.Fatal(err)
	}
	empty := native.Request{Version: 1, Mode: "adopt", Profile: "base", Entries: []string{}}
	p, err = e.Plan(context.Background(), empty)
	if err != nil {
		t.Fatal(err)
	}
	if err = e.Apply(context.Background(), p, native.Consent{Approve: p.ID}); err != nil {
		t.Fatal(err)
	}
	a, err := e.LoadAdoption()
	if err != nil || len(a.Deselected) != 2 || len(a.Customs) != 1 {
		t.Fatalf("local persistence %+v %v", a, err)
	}
	req.Metadata = nil
	p, err = e.Plan(context.Background(), req)
	if err != nil || p.Selected["prefs"].Tools[0] != "local-editor" {
		t.Fatalf("reselection %+v %v", p, err)
	}
	after, _ := os.ReadFile(filepath.Join(c.Source, "manifest.json"))
	if string(before) != string(after) {
		t.Fatal("Adopt rewrote shared manifest")
	}
}
func TestConflictingAdoptionStateFailsClosed(t *testing.T) {
	e, c := fixture(t)
	a, _ := e.LoadAdoption()
	a.Deselected = []string{"prefs"}
	a.Entries["prefs"] = native.Entry{Source: "home/prefs", Destination: "prefs", OS: []string{"linux"}, Impact: "Safe", Portability: "managed-shared"}
	writeJSON(t, filepath.Join(c.State, "adoption.json"), a)
	if _, err := e.Plan(context.Background(), native.Request{Version: 1, Mode: "adopt", Profile: "base"}); err == nil {
		t.Fatal("accepted conflicting selected/deselected state")
	}
}
func TestRecoveryRollsBackOnlyExactInterruptedActions(t *testing.T) {
	e, c := fixture(t)
	write(t, filepath.Join(c.Home, "prefs"), "old", 0640)
	write(t, filepath.Join(c.Home, "unrelated"), "keep", 0600)
	p, err := e.Plan(context.Background(), native.Request{Version: 1, Mode: "adopt", Profile: "base"})
	if err != nil {
		t.Fatal(err)
	}
	writeJSON(t, filepath.Join(c.State, "recovery.json"), native.Journal{Plan: p, Actions: p.Actions})
	write(t, filepath.Join(c.Home, "prefs"), string(p.Actions[0].After.Data), p.Actions[0].After.Mode)
	write(t, filepath.Join(c.Home, "prefs.dot-backup"), "old", 0640)
	if err = e.Apply(context.Background(), p, native.Consent{Approve: p.ID}); err == nil {
		t.Fatal("apply allowed interrupted recovery")
	}
	if err = e.Recover(context.Background(), "wrong"); err == nil {
		t.Fatal("recovery lacked exact consent")
	}
	if err = e.Recover(context.Background(), p.ID); err != nil {
		t.Fatal(err)
	}
	b, _ := os.ReadFile(filepath.Join(c.Home, "prefs"))
	u, _ := os.ReadFile(filepath.Join(c.Home, "unrelated"))
	if string(b) != "old" || string(u) != "keep" {
		t.Fatal("unscoped rollback")
	}
	if _, err = os.Stat(filepath.Join(c.State, "recovery.json")); !os.IsNotExist(err) {
		t.Fatal("journal not cleared")
	}
}
func TestStaleSourceTargetStateAndBackupRejectBeforeMutation(t *testing.T) {
	for _, change := range []string{"source", "target", "state", "backup", "plan"} {
		t.Run(change, func(t *testing.T) {
			e, c := fixture(t)
			write(t, filepath.Join(c.Home, "prefs"), "old", 0600)
			write(t, filepath.Join(c.Home, "prefs.dot-backup"), "older", 0600)
			p, err := e.Plan(context.Background(), native.Request{Version: 1, Mode: "adopt", Profile: "base"})
			if err != nil {
				t.Fatal(err)
			}
			switch change {
			case "source":
				write(t, filepath.Join(c.Source, "home/prefs"), "changed", 0751)
			case "target":
				write(t, filepath.Join(c.Home, "prefs"), "changed", 0600)
			case "state":
				a, _ := e.LoadAdoption()
				a.Customs = map[string]map[string]any{"prefs": {"tools": []string{"new"}}}
				writeJSON(t, filepath.Join(c.State, "adoption.json"), a)
			case "backup":
				write(t, filepath.Join(c.Home, "prefs.dot-backup"), "changed", 0600)
			case "plan":
				p.Actions[0].Path = "unrelated"
			}
			before, _ := os.ReadFile(filepath.Join(c.Home, "prefs"))
			if err = e.Apply(context.Background(), p, native.Consent{Approve: p.ID, Backup: true}); err == nil {
				t.Fatal("stale plan accepted")
			}
			after, _ := os.ReadFile(filepath.Join(c.Home, "prefs"))
			if string(before) != string(after) {
				t.Fatal("stale apply mutated")
			}
		})
	}
}
func TestUnsafeDesktopAndBackupRequireDistinctConsent(t *testing.T) {
	e, c := fixture(t)
	var m native.Manifest
	b, _ := os.ReadFile(filepath.Join(c.Source, "manifest.json"))
	_ = native.Decode(b, &m)
	x := m.Entries["prefs"]
	x.Impact = "Unsafe"
	x.Integrations = []string{"desktop"}
	m.Entries["prefs"] = x
	writeJSON(t, filepath.Join(c.Source, "manifest.json"), m)
	write(t, filepath.Join(c.Home, "prefs"), "old", 0600)
	write(t, filepath.Join(c.Home, "prefs.dot-backup"), "older", 0600)
	p, err := e.Plan(context.Background(), native.Request{Version: 1, Mode: "adopt", Profile: "base"})
	if err != nil {
		t.Fatal(err)
	}
	if !p.Gates.Unsafe || !p.Gates.Desktop || !p.Gates.Backup {
		t.Fatal("missing separate gates")
	}
	for _, consent := range []native.Consent{{Approve: p.ID}, {Approve: p.ID, Unsafe: true}, {Approve: p.ID, Unsafe: true, Desktop: true}} {
		if err = e.Apply(context.Background(), p, consent); err == nil {
			t.Fatal("gate bypass")
		}
	}
	if err = e.Apply(context.Background(), p, native.Consent{Approve: p.ID, Unsafe: true, Desktop: true, Backup: true}); err != nil {
		t.Fatal(err)
	}
	b, _ = os.ReadFile(filepath.Join(c.Home, "prefs.dot-backup"))
	if string(b) != "old" {
		t.Fatal("backup wrong")
	}
}
func TestContractAndUnsafePathsFailBeforeWrites(t *testing.T) {
	for _, bad := range []string{"protocol", "unknown-assignment", "empty-assignment", "wildcard", "source-escape", "target-escape", "symlink", "whole-git", "state-target"} {
		t.Run(bad, func(t *testing.T) {
			e, c := fixture(t)
			var m native.Manifest
			b, _ := os.ReadFile(filepath.Join(c.Source, "manifest.json"))
			_ = native.Decode(b, &m)
			x := m.Entries["prefs"]
			switch bad {
			case "protocol":
				var d native.Contract
				b, _ := os.ReadFile(filepath.Join(c.Source, "dot-contract.json"))
				_ = native.Decode(b, &d)
				d.Protocol = 2
				writeJSON(t, filepath.Join(c.Source, "dot-contract.json"), d)
			case "unknown-assignment":
				write(t, filepath.Join(c.Source, "manifest.json"), strings.Replace(string(b), `"impact":"Safe"`, `"impact":"Safe","machines":{"include":["all"],"exclude":[],"future":true}`, 1), 0600)
			case "empty-assignment":
				x.Machines = &native.Assignment{Include: []string{}, Exclude: []string{}}
			case "wildcard":
				x.Machines = &native.Assignment{Include: []string{"*"}, Exclude: []string{}}
			case "source-escape":
				x.Source = "../outside"
			case "target-escape":
				x.Destination = "../outside"
			case "whole-git":
				x.Destination = ".gitconfig"
			case "symlink":
				if err := os.Symlink(filepath.Join(c.Source, "home/prefs"), filepath.Join(c.Home, "prefs")); err != nil {
					t.Fatal(err)
				}
			case "state-target":
				c.State = filepath.Join(c.Home, "local-state")
				if err := os.Mkdir(c.State, 0700); err != nil {
					t.Fatal(err)
				}
				e2, err := native.New(c)
				if err != nil {
					t.Fatal(err)
				}
				e = e2
				if err = e.SaveIdentity(native.Machine{ID: "linux_test", OS: "linux"}); err != nil {
					t.Fatal(err)
				}
				x.Destination = "local-state/adoption.json"
			}
			if !hasSpecial(bad) {
				m.Entries["prefs"] = x
				writeJSON(t, filepath.Join(c.Source, "manifest.json"), m)
			}
			before, _ := os.ReadFile(filepath.Join(c.State, "adoption.json"))
			if _, err := e.Plan(context.Background(), native.Request{Version: 1, Mode: "adopt", Profile: "base"}); err == nil {
				t.Fatal("unsafe contract accepted")
			}
			after, _ := os.ReadFile(filepath.Join(c.State, "adoption.json"))
			if string(before) != string(after) {
				t.Fatal("invalid plan mutated state")
			}
		})
	}
}
func hasSpecial(s string) bool { return s == "protocol" || s == "unknown-assignment" || s == "symlink" }
func TestExplicitPullPushAndMachineTwoImport(t *testing.T) {
	e, c := fixture(t)
	git(t, c.Source, "add", ".")
	git(t, c.Source, "-c", "user.name=Fixture", "-c", "user.email=test@example.invalid", "commit", "-qm", "initial")
	bare := filepath.Join(filepath.Dir(c.Source), "remote.git")
	git(t, c.Source, "init", "--bare", bare)
	git(t, c.Source, "remote", "add", "origin", bare)
	if err := e.Push(context.Background(), false); err == nil {
		t.Fatal("push without explicit consent")
	}
	if err := e.Push(context.Background(), true); err != nil {
		t.Fatal(err)
	}
	second := filepath.Join(filepath.Dir(c.Source), "second")
	git(t, c.Source, "clone", "-q", bare, second)
	home2 := filepath.Join(filepath.Dir(c.Home), "home2")
	state2 := filepath.Join(filepath.Dir(c.State), "state2")
	_ = os.Mkdir(home2, 0700)
	e2, err := native.New(native.Config{Source: second, Home: home2, State: state2, OS: "linux"})
	if err != nil {
		t.Fatal(err)
	}
	if err = e2.SaveIdentity(native.Machine{ID: "linux_second", OS: "linux"}); err != nil {
		t.Fatal(err)
	}
	write(t, filepath.Join(c.Source, "home/prefs"), "remote changed\n", 0751)
	git(t, c.Source, "add", "home/prefs")
	git(t, c.Source, "-c", "user.name=Fixture", "-c", "user.email=test@example.invalid", "commit", "-qm", "change")
	if err = e.Push(context.Background(), true); err != nil {
		t.Fatal(err)
	}
	if err = e2.Pull(context.Background(), false); err == nil {
		t.Fatal("pull without explicit consent")
	}
	if err = e2.Pull(context.Background(), true); err != nil {
		t.Fatal(err)
	}
	if _, err = os.Stat(filepath.Join(home2, "prefs")); !os.IsNotExist(err) {
		t.Fatal("pull auto-adopted")
	}
	diffs, err := e2.Diff(context.Background())
	if err != nil || len(diffs) != 1 || !strings.Contains(diffs[0].Text, "remote changed") {
		t.Fatalf("import diff %+v %v", diffs, err)
	}
	p, err := e2.Plan(context.Background(), native.Request{Version: 1, Mode: "adopt", Profile: "base", Entries: []string{"prefs"}})
	if err != nil {
		t.Fatal(err)
	}
	if err = e2.Apply(context.Background(), p, native.Consent{Approve: p.ID}); err != nil {
		t.Fatal(err)
	}
	a, _ := e.LoadAdoption()
	a2, _ := e2.LoadAdoption()
	if a.Machine.ID != "linux_test" || len(a.Entries) != 0 || a2.Machine.ID != "linux_second" || len(a2.Entries) != 1 {
		t.Fatal("cross-machine state mutation")
	}
	write(t, filepath.Join(second, "unrelated"), "dirty", 0600)
	if err = e2.Pull(context.Background(), true); err == nil {
		t.Fatal("dirty checkout should require inspection")
	}
	b, _ := os.ReadFile(filepath.Join(second, "unrelated"))
	if string(b) != "dirty" {
		t.Fatal("discarded unrelated work")
	}
}
func TestLogicalTrackRetargetAndMachineLayerResolution(t *testing.T) {
	e, c := fixture(t)
	var d native.Contract
	b, _ := os.ReadFile(filepath.Join(c.Source, "dot-contract.json"))
	_ = native.Decode(b, &d)
	d.ManifestVersion = 2
	writeJSON(t, filepath.Join(c.Source, "dot-contract.json"), d)
	var m native.Manifest
	b, _ = os.ReadFile(filepath.Join(c.Source, "manifest.json"))
	_ = native.Decode(b, &m)
	m.Version = 2
	m.Machines = map[string]native.MachineInfo{"linux_test": {OS: "linux"}, "linux_other": {OS: "linux"}}
	m.Tracking = map[string]native.Logical{}
	writeJSON(t, filepath.Join(c.Source, "manifest.json"), m)
	write(t, filepath.Join(c.Home, "logical"), "logical bytes", 0640)
	req := native.Request{Version: 1, Mode: "track", Profile: "logical", Selections: []native.Selection{{Path: "logical", Logical: "logical", Destinations: map[string]string{"linux": "logical"}, Common: map[string]any{"tools": []string{"common"}}, Machine: map[string]map[string]any{"linux_test": {"tools": []string{"machine"}}}, OSMetadata: map[string]map[string]any{}}}}
	p, err := e.Plan(context.Background(), req)
	if err != nil {
		t.Fatal(err)
	}
	if p.Selected["logical@linux"].Machines.Include[0] != "all" {
		t.Fatal("default assignment missing")
	}
	if err = e.Apply(context.Background(), p, native.Consent{Approve: p.ID, Backup: true}); err != nil {
		t.Fatal(err)
	}
	meta, err := e.Plan(context.Background(), native.Request{Version: 1, Mode: "metadata", Profile: "logical", Metadata: map[string]map[string]any{"logical": {"common": map[string]any{"tools": []string{"reviewed-common"}}}}})
	if err != nil {
		t.Fatal(err)
	}
	if err = e.Apply(context.Background(), meta, native.Consent{Approve: meta.ID, Backup: true}); err != nil {
		t.Fatal(err)
	}
	c2 := c
	c2.Home = filepath.Join(filepath.Dir(c.Home), "other-home")
	c2.State = filepath.Join(filepath.Dir(c.State), "other-state")
	_ = os.Mkdir(c2.Home, 0700)
	e2, err := native.New(c2)
	if err != nil {
		t.Fatal(err)
	}
	if err = e2.SaveIdentity(native.Machine{ID: "linux_other", OS: "linux"}); err != nil {
		t.Fatal(err)
	}
	other, err := e2.Plan(context.Background(), native.Request{Version: 1, Mode: "adopt", Profile: "logical"})
	if err != nil || other.Selected["logical@linux"].Tools[0] != "reviewed-common" {
		t.Fatalf("leaked machine override %+v %v", other.Selected, err)
	}
	p, err = e.Plan(context.Background(), native.Request{Version: 1, Mode: "adopt", Profile: "logical"})
	if err != nil || p.Selected["logical@linux"].Tools[0] != "machine" {
		t.Fatalf("machine metadata %+v %v", p.Selected, err)
	}
	req.Selections[0].Machines = &native.Assignment{Include: []string{"linux_other"}, Exclude: []string{}}
	p, err = e.Plan(context.Background(), req)
	if err != nil {
		t.Fatal(err)
	}
	if err = e.Apply(context.Background(), p, native.Consent{Approve: p.ID, Backup: true}); err != nil {
		t.Fatal(err)
	}
	p, err = e.Plan(context.Background(), native.Request{Version: 1, Mode: "adopt", Profile: "logical"})
	if err != nil || len(p.Selected) != 0 {
		t.Fatalf("retargeted scope %+v %v", p.Selected, err)
	}
}
func TestBoundedGitTrackRetargetPreservesOtherApprovedFieldsAndRedactsSecrets(t *testing.T) {
	e, c := fixture(t)
	var m native.Manifest
	b, _ := os.ReadFile(filepath.Join(c.Source, "manifest.json"))
	_ = native.Decode(b, &m)
	m.Entries["git"] = native.Entry{Source: "home/git", Destination: ".gitconfig", OS: []string{"linux"}, Portability: "managed-templated", Impact: "Unsafe", Integrations: []string{"git-config"}, GitKeys: []string{"user.name", "user.email"}, GitValues: map[string]string{"user.name": "Old", "user.email": "approved@example.invalid"}}
	m.Profiles["git"] = []string{"git"}
	writeJSON(t, filepath.Join(c.Source, "manifest.json"), m)
	live := "[user]\nname=Local\nemail=not-selected@example.invalid\n[credential]\npassword=private-value\n"
	write(t, filepath.Join(c.Home, ".gitconfig"), live, 0600)
	p, err := e.Plan(context.Background(), native.Request{Version: 1, Mode: "track", Profile: "git", Selections: []native.Selection{{Path: ".gitconfig", Entry: native.Entry{GitKeys: []string{"user.name"}, Machines: &native.Assignment{Include: []string{"linux_other"}, Exclude: []string{}}}}}})
	if err != nil {
		t.Fatal(err)
	}
	if len(p.Selected["git"].GitKeys) != 2 || p.Selected["git"].GitValues["user.email"] != "approved@example.invalid" {
		t.Fatal("retarget changed another approved field")
	}
	encoded, _ := json.Marshal(p)
	if strings.Contains(string(encoded), "private-value") || strings.Contains(string(encoded), "not-selected@example.invalid") {
		t.Fatal("unapproved Git data leaked")
	}
	if err = e.Apply(context.Background(), p, native.Consent{Approve: p.ID, Unsafe: true, Backup: true}); err != nil {
		t.Fatal(err)
	}
	b, _ = os.ReadFile(filepath.Join(c.Home, ".gitconfig"))
	if string(b) != live {
		t.Fatal("Track mutated live Git config")
	}
}
func TestDeclarativeTemplatesRenderWithoutExecutingPrivateHelpers(t *testing.T) {
	e, c := fixture(t)
	var m native.Manifest
	b, _ := os.ReadFile(filepath.Join(c.Source, "manifest.json"))
	_ = native.Decode(b, &m)
	x := m.Entries["prefs"]
	x.Portability = "managed-templated"
	m.Entries["prefs"] = x
	writeJSON(t, filepath.Join(c.Source, "manifest.json"), m)
	c.TrackerConfig = filepath.Join(filepath.Dir(c.State), "chezmoi.toml")
	write(t, c.TrackerConfig, "[data]\nname = \"Example Name\"\n", 0600)
	e, _ = native.New(c)
	write(t, filepath.Join(c.Source, "home/prefs"), "{{ .name }} on {{ .chezmoi.os }} / {{ .machine.ID }}\n", 0751)
	p, err := e.Plan(context.Background(), native.Request{Version: 1, Mode: "adopt", Profile: "base"})
	if err != nil {
		t.Fatal(err)
	}
	if string(p.Actions[0].After.Data) != "Example Name on linux / linux_test\n" {
		t.Fatal("template data not resolved")
	}
	write(t, filepath.Join(c.Source, "home/prefs"), `{{ output "sh" "-c" "touch should-not-exist" }}`, 0751)
	if _, err = e.Plan(context.Background(), native.Request{Version: 1, Mode: "adopt", Profile: "base"}); err == nil {
		t.Fatal("accepted private command template")
	}
}
func TestUntrackLeavesTargetSourceBytesAndLocalApprovalUntouched(t *testing.T) {
	e, c := fixture(t)
	p, err := e.Plan(context.Background(), native.Request{Version: 1, Mode: "adopt", Profile: "base"})
	if err != nil {
		t.Fatal(err)
	}
	if err = e.Apply(context.Background(), p, native.Consent{Approve: p.ID}); err != nil {
		t.Fatal(err)
	}
	before, _ := os.ReadFile(filepath.Join(c.State, "adoption.json"))
	p, err = e.Plan(context.Background(), native.Request{Version: 1, Mode: "untrack", Profile: "base", Entries: []string{"prefs"}})
	if err != nil {
		t.Fatal(err)
	}
	if err = e.Apply(context.Background(), p, native.Consent{Approve: p.ID, Backup: true}); err != nil {
		t.Fatal(err)
	}
	after, _ := os.ReadFile(filepath.Join(c.State, "adoption.json"))
	target, _ := os.ReadFile(filepath.Join(c.Home, "prefs"))
	source, _ := os.ReadFile(filepath.Join(c.Source, "home/prefs"))
	if string(before) != string(after) || string(target) != "shared\n" || string(source) != "shared\n" {
		t.Fatal("untrack discarded content/approval")
	}
	if _, err = e.Plan(context.Background(), native.Request{Version: 1, Mode: "update", Profile: "base"}); err == nil {
		t.Fatal("update silently broadened/repaired missing enrollment")
	}
}
func TestRecoveryRefusesPostInterruptionEdits(t *testing.T) {
	e, c := fixture(t)
	p, err := e.Plan(context.Background(), native.Request{Version: 1, Mode: "adopt", Profile: "base"})
	if err != nil {
		t.Fatal(err)
	}
	writeJSON(t, filepath.Join(c.State, "recovery.json"), native.Journal{Plan: p, Actions: p.Actions})
	write(t, filepath.Join(c.Home, "prefs"), "independent edit", 0600)
	if err = e.Recover(context.Background(), p.ID); err == nil {
		t.Fatal("recovery overwrote independent edit")
	}
	b, _ := os.ReadFile(filepath.Join(c.Home, "prefs"))
	if string(b) != "independent edit" {
		t.Fatal("recovery mutated conflict")
	}
}
func TestExactSkillsMirrorsAndRemovalPreserveUnrelatedFiles(t *testing.T) {
	e, c := fixture(t)
	var m native.Manifest
	b, _ := os.ReadFile(filepath.Join(c.Source, "manifest.json"))
	_ = native.Decode(b, &m)
	m.Entries["skill"] = native.Entry{Source: "home/skill", Destination: ".agents/skills/example/SKILL.md", OS: []string{"linux"}, Portability: "managed-shared", Impact: "Safe", Removals: []string{"obsolete"}, Integrations: []string{"gemini-skills"}}
	m.Profiles["skills"] = []string{"skill"}
	writeJSON(t, filepath.Join(c.Source, "manifest.json"), m)
	write(t, filepath.Join(c.Source, "home/skill"), "reviewed skill\n", 0640)
	_ = os.MkdirAll(filepath.Join(c.Home, ".gemini/skills/custom"), 0700)
	write(t, filepath.Join(c.Home, ".gemini/skills/custom/SKILL.md"), "unapproved", 0600)
	write(t, filepath.Join(c.Home, "obsolete"), "old exact", 0700)
	p, err := e.Plan(context.Background(), native.Request{Version: 1, Mode: "adopt", Profile: "skills", Entries: []string{"skill"}})
	if err != nil {
		t.Fatal(err)
	}
	if err = e.Apply(context.Background(), p, native.Consent{Approve: p.ID}); err != nil {
		t.Fatal(err)
	}
	for _, path := range []string{".agents/skills/example/SKILL.md", ".gemini/skills/example/SKILL.md"} {
		b, _ = os.ReadFile(filepath.Join(c.Home, path))
		if string(b) != "reviewed skill\n" {
			t.Fatal("missing exact skill integration")
		}
	}
	b, _ = os.ReadFile(filepath.Join(c.Home, ".gemini/skills/custom/SKILL.md"))
	if string(b) != "unapproved" {
		t.Fatal("unscoped mirror cleanup")
	}
	b, _ = os.ReadFile(filepath.Join(c.Home, "obsolete.dot-backup"))
	if string(b) != "old exact" {
		t.Fatal("removed bytes not backed up")
	}
	if _, err = os.Stat(filepath.Join(c.Home, "obsolete")); !os.IsNotExist(err) {
		t.Fatal("exact removal not applied")
	}
}
func TestSnapshotNeverEnrollsFutureFiles(t *testing.T) {
	e, c := fixture(t)
	_ = os.Mkdir(filepath.Join(c.Home, "folder"), 0700)
	write(t, filepath.Join(c.Home, "folder/one"), "one", 0600)
	snapshot, err := e.Snapshot(context.Background(), "folder")
	if err != nil || len(snapshot.Paths) != 1 || snapshot.Paths[0] != "folder/one" {
		t.Fatalf("snapshot %+v %v", snapshot, err)
	}
	write(t, filepath.Join(c.Home, "folder/two"), "future", 0600)
	p, err := e.Plan(context.Background(), native.Request{Version: 1, Mode: "track", Profile: "folder", Selections: []native.Selection{{Path: snapshot.Paths[0], Entry: native.Entry{OS: []string{"linux"}, Portability: "managed-shared", Impact: "Safe"}}}})
	if err != nil {
		t.Fatal(err)
	}
	if len(p.Selected) != 1 || p.Selected["folder/one"].Destination != "folder/one" {
		t.Fatal("future file gained implicit consent")
	}
}
func TestStrictRequestDecoderRejectsAmbiguousFields(t *testing.T) {
	for _, body := range []string{`{"version":1,"version":1,"mode":"adopt","profile":"base"}`, `{"version":1,"mode":"track","profile":"base","selections":[{"path":"prefs","machines":null}]}`, `{"version":1,"mode":"track","profile":"base","selections":[{"path":"prefs","machines":{"include":["all"],"exclude":[],"future":true}}]}`, `{"version":1,"mode":"adopt","profile":"base","future":true}`} {
		var r native.Request
		if err := native.Decode([]byte(body), &r); err == nil {
			t.Fatalf("ambiguous request accepted: %s", body)
		}
	}
}

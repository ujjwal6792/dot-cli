package setup_test

import (
	tea "charm.land/bubbletea/v2"
	"context"
	"encoding/json"
	"flag"
	"io"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/ujjwal6792/dots-cli/internal/native"
	"github.com/ujjwal6792/dots-cli/internal/setup"
	"github.com/ujjwal6792/dots-cli/internal/tui"
)

func local(t *testing.T, o setup.Options) *setup.Engine {
	t.Helper()
	e, err := setup.New(o)
	if err != nil {
		t.Fatal(err)
	}
	if err := e.CreateSource(context.Background(), "new", "https://example.invalid/dots.git"); err != nil {
		t.Fatal(err)
	}
	return e
}
func TestNarrowSetupCanScrollEntireDependencyTableAndNamedBackup(t *testing.T) {
	o := options(t)
	o.Git = filepath.Join(o.Home, "missing-git")
	o.Chezmoi = filepath.Join(o.Home, "missing-chezmoi")
	o.Brew = "/usr/bin/false"
	e, err := setup.New(o)
	if err != nil {
		t.Fatal(err)
	}
	m := tui.NewInitModel(e)
	m, _ = m.Update(tea.WindowSizeMsg{Width: 40, Height: 12})
	m = step(m, "enter")
	for i := 0; i < 30; i++ {
		m = step(m, "J")
	}
	if !strings.Contains(m.View().Content, "chezmoi.io") || !strings.Contains(m.View().Content, "Enter Install") {
		t.Fatal("dependency table inaccessible: " + m.View().Content)
	}
	o = options(t)
	e = local(t, o)
	m = tui.NewInitModel(e)
	m, _ = m.Update(tea.WindowSizeMsg{Width: 40, Height: 12})
	m = step(m, "enter")
	m = step(m, "j")
	m = step(m, "enter")
	for i := 0; i < 30; i++ {
		m = step(m, "J")
	}
	if !strings.Contains(m.View().Content, "checkout") || !strings.Contains(m.View().Content, "> Cancel") {
		t.Fatal("named backup inaccessible/default not cancel: " + m.View().Content)
	}
	if _, err := os.Stat(o.Source); err != nil {
		t.Fatal("scrolling replaced source")
	}
}
func TestSetupFlagBindingAndAppConstructionDoNotAuthorizeWrites(t *testing.T) {
	o := options(t)
	var parsed setup.Options
	fs := flag.NewFlagSet("init", flag.ContinueOnError)
	fs.SetOutput(io.Discard)
	parsed.BindFlags(fs)
	if err := fs.Parse([]string{"--home", o.Home, "--source", o.Source, "--config", o.Config, "--state-dir", o.State, "--git", o.Git, "--name", "Fixture", "--email", "fixture@example.invalid"}); err != nil {
		t.Fatal(err)
	}
	m, e, err := setup.NewApp(context.Background(), parsed)
	if err != nil {
		t.Fatal(err)
	}
	if m.Init() != nil || !strings.Contains(m.View().Content, "Welcome to dot") {
		t.Fatal("first-run app constructor not welcome/read-only")
	}
	if e.Options().Name != "Fixture" || e.Options().State != o.State {
		t.Fatal(e.Options())
	}
	if _, err := os.Stat(o.Source); !os.IsNotExist(err) {
		t.Fatal("flag/app constructor wrote source")
	}
	if _, err := os.Stat(o.State); !os.IsNotExist(err) {
		t.Fatal("flag/app constructor wrote state")
	}
	if err := fs.Parse([]string{"--profile", "personal"}); err == nil {
		t.Fatal("retired profile flag accepted")
	}
}
func TestMissingGitOffersDependenciesBeforeAnySourceCreation(t *testing.T) {
	o := options(t)
	o.Git = filepath.Join(o.Home, "missing-git")
	e, err := setup.New(o)
	if err != nil {
		t.Fatal(err)
	}
	m := step(tui.NewInitModel(e), "enter")
	if !strings.Contains(m.View().Content, "Dependencies") || !strings.Contains(m.View().Content, "git-scm.com") {
		t.Fatal(m.View().Content)
	}
	if _, err := os.Stat(o.Source); !os.IsNotExist(err) {
		t.Fatal("missing Git created source")
	}
	if err := e.CreateSource(context.Background(), "new", "https://example.invalid/dots.git"); err == nil {
		t.Fatal("source succeeded without Git")
	}
	if _, err := os.Stat(filepath.Dir(o.Source)); !os.IsNotExist(err) {
		t.Fatal("missing Git wrote parent")
	}
}
func TestExplicitHomeControlsGitIdentityAndCanceledSourceHasNoWrites(t *testing.T) {
	o := options(t)
	e := local(t, o)
	global := filepath.Join(o.Home, ".gitconfig")
	if err := os.WriteFile(global, []byte("[user]\n name = Explicit Home\n email = explicit@example.invalid\n"), 0600); err != nil {
		t.Fatal(err)
	}
	t.Setenv("GIT_CONFIG_GLOBAL", "") // Register restoration, then remove the override.
	if err := os.Unsetenv("GIT_CONFIG_GLOBAL"); err != nil {
		t.Fatal(err)
	}
	if err := e.SaveMachine(context.Background(), "macos_fixture"); err != nil {
		t.Fatal(err)
	}
	p, err := e.PlanTracker(context.Background(), "Wrong", "wrong@example.invalid")
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(p.Text, "Explicit Home") || strings.Contains(p.Text, "Wrong") {
		t.Fatal("Git read a different HOME: " + p.Text)
	}
	other := options(t)
	fresh, err := setup.New(other)
	if err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	if err := fresh.CreateSource(ctx, "new", "https://example.invalid/dots.git"); err == nil {
		t.Fatal("canceled operation succeeded")
	}
	if _, err := os.Stat(filepath.Dir(other.Source)); !os.IsNotExist(err) {
		t.Fatal("canceled operation wrote source parent")
	}
}
func TestDefaultAndCustomNestedSourcesNeverBecomeTargetEnrollment(t *testing.T) {
	for _, nested := range []string{"", "repos/private-dots"} {
		t.Run(nested, func(t *testing.T) {
			for _, env := range []string{"DOT_SOURCE", "DOT_CONFIG", "DOT_STATE_DIR", "XDG_STATE_HOME", "XDG_CONFIG_HOME"} {
				t.Setenv(env, "")
			}
			o := options(t)
			o.Source = ""
			o.State = ""
			o.Config = ""
			if nested != "" {
				o.Source = filepath.Join(o.Home, nested)
			}
			e := local(t, o)
			resolved := e.Options()
			if nested == "" && resolved.Source != filepath.Join(o.Home, ".local/share/dot-config") {
				t.Fatal("default relocated")
			}
			if err := e.SaveMachine(context.Background(), "macos_fixture"); err != nil {
				t.Fatal(err)
			}
			for _, p := range []string{filepath.Join(resolved.Source, "manifest.json"), filepath.Join(resolved.State, "adoption.json"), resolved.Config} {
				rel, _ := filepath.Rel(o.Home, p)
				_, err := e.Policy.Plan(context.Background(), native.Request{Version: 1, Mode: "track", Profile: "fixture", Selections: []native.Selection{{Path: rel, Entry: native.Entry{Destination: rel, OS: []string{"darwin"}, Portability: "managed-shared", Impact: "Safe"}}}})
				if err == nil {
					t.Fatal("private path enrolled: " + rel)
				}
			}
		})
	}
}
func TestStateAndConnectionAreStrictAndNeverFallBackToOtherMachine(t *testing.T) {
	o := options(t)
	e := local(t, o)
	if err := e.SaveMachine(context.Background(), "macos_first"); err != nil {
		t.Fatal(err)
	}
	p, err := e.PlanTracker(context.Background(), "Fixture", "fixture@example.invalid")
	if err != nil {
		t.Fatal(err)
	}
	if err := e.ApplyTracker(context.Background(), p, p.ID, true); err != nil {
		t.Fatal(err)
	}
	first, _ := os.ReadFile(filepath.Join(o.State, "adoption.json"))
	isolated := o
	isolated.State = filepath.Join(filepath.Dir(o.State), "machine-two")
	isolated.Config = filepath.Join(filepath.Dir(o.Config), "machine-two.toml")
	second, err := setup.New(isolated)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := second.ReadConnection(); !os.IsNotExist(err) {
		t.Fatalf("other state connection guessed: %v", err)
	}
	if err := second.SaveMachine(context.Background(), "macos_second"); err != nil {
		t.Fatal(err)
	}
	after, _ := os.ReadFile(filepath.Join(o.State, "adoption.json"))
	if string(first) != string(after) {
		t.Fatal("other machine changed")
	}
	for _, record := range []string{`{"version":1}`, `{"version":1,"version":1}`, `{"version":99,"source":"/secret"}`, `null`, `{bad`} {
		state := filepath.Join(filepath.Dir(o.State), "bad-state")
		if err := os.MkdirAll(state, 0700); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(state, "connection.json"), []byte(record), 0600); err != nil {
			t.Fatal(err)
		}
		bad := o
		bad.State = state
		if _, err := setup.New(bad); err == nil {
			t.Fatal("malformed record accepted: " + record)
		}
	}
	conflict := o
	conflict.Source = filepath.Join(o.Home, "different-source")
	if _, err := setup.New(conflict); err == nil {
		t.Fatal("connection silently switched paths")
	}
}
func TestSymlinkSourcesStateConnectionAndBackupAreRejected(t *testing.T) {
	o := options(t)
	e := local(t, o)
	if err := os.Symlink("manifest.json", filepath.Join(o.Source, "linked")); err != nil {
		t.Fatal(err)
	}
	if _, err := e.PlanBackup(); err == nil {
		t.Fatal("symlinked source data accepted for replacement")
	}
	actual := filepath.Join(filepath.Dir(o.State), "actual-state")
	if err := os.Mkdir(actual, 0700); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(actual, o.State); err != nil {
		t.Fatal(err)
	}
	if _, err := setup.New(o); err == nil {
		t.Fatal("symlinked state accepted")
	}
	fresh := options(t)
	if err := os.Mkdir(fresh.State, 0700); err != nil {
		t.Fatal(err)
	}
	record := filepath.Join(filepath.Dir(fresh.State), "other-connection")
	if err := os.WriteFile(record, []byte(`{}`), 0600); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(record, filepath.Join(fresh.State, "connection.json")); err != nil {
		t.Fatal(err)
	}
	if _, err := setup.New(fresh); err == nil {
		t.Fatal("symlinked connection accepted")
	}
}
func TestTrackerStalenessAndConflictsCannotReachDone(t *testing.T) {
	o := options(t)
	e := local(t, o)
	if err := e.SaveMachine(context.Background(), "macos_fixture"); err != nil {
		t.Fatal(err)
	}
	p, err := e.PlanTracker(context.Background(), "Fixture", "fixture@example.invalid")
	if err != nil {
		t.Fatal(err)
	}
	git(t, o.Source, "config", "user.name", "Changed")
	if err := e.ApplyTracker(context.Background(), p, p.ID, true); err == nil {
		t.Fatal("stale Git identity accepted")
	}
	if _, err := os.Stat(o.Config); !os.IsNotExist(err) {
		t.Fatal("stale plan wrote tracker")
	}
	p, err = e.PlanTracker(context.Background(), "Fixture", "fixture@example.invalid")
	if err != nil {
		t.Fatal(err)
	}
	if err := os.MkdirAll(filepath.Dir(o.Config), 0700); err != nil {
		t.Fatal(err)
	}
	text := strings.Replace(p.Text, o.Home, "/different-home", 1)
	if err := os.WriteFile(o.Config, []byte(text), 0600); err != nil {
		t.Fatal(err)
	}
	if _, err := e.PlanTracker(context.Background(), "", ""); err == nil {
		t.Fatal("tracker identity conflict accepted")
	}
	if err := e.Complete(context.Background()); err == nil {
		t.Fatal("conflicting tracker falsely complete")
	}
	if b, _ := os.ReadFile(o.Config); string(b) != text {
		t.Fatal("conflicting tracker modified")
	}
}
func TestMissingDependenciesAndSkipCannotReachDone(t *testing.T) {
	o := options(t)
	o.Chezmoi = filepath.Join(o.Home, "missing-chezmoi")
	e := local(t, o)
	if err := e.SaveMachine(context.Background(), "macos_fixture"); err != nil {
		t.Fatal(err)
	}
	m := tui.NewInitModel(e)
	m = step(m, "enter")
	m = step(m, "enter")
	m = step(m, "enter")
	if !strings.Contains(m.View().Content, "Dependencies") {
		t.Fatal(m.View().Content)
	}
	m = step(m, "space")
	if atDone(m) || !strings.Contains(m.View().Content, "Setup incomplete") {
		t.Fatal(m.View().Content)
	}
	if _, err := os.Stat(o.Config); !os.IsNotExist(err) {
		t.Fatal("skip created tracker")
	}
	if err := e.Complete(context.Background()); err == nil {
		t.Fatal("missing dependency falsely complete")
	}
}
func TestLinuxMachineSuggestionUsesOSReleaseOfflineWithoutChangingCatalog(t *testing.T) {
	o := options(t)
	o.OS = "linux"
	o.User = "Some.User"
	o.OSRelease = filepath.Join(o.Home, "os-release")
	if err := os.WriteFile(o.OSRelease, []byte("ID_LIKE=\"Arch Linux\"\n"), 0600); err != nil {
		t.Fatal(err)
	}
	e := local(t, o)
	m, err := e.SuggestMachine(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if m.ID != "arch_linux_some_user" || m.OS != "linux" {
		t.Fatal(m)
	}
	if _, err := os.Stat(o.State); !os.IsNotExist(err) {
		t.Fatal("suggestion wrote state")
	}
}
func TestExistingTrackerPreservesExtraDataAndMalformedTOMLIsSecretSafe(t *testing.T) {
	o := options(t)
	e := local(t, o)
	if err := e.SaveMachine(context.Background(), "macos_fixture"); err != nil {
		t.Fatal(err)
	}
	p, err := e.PlanTracker(context.Background(), "Fixture", "fixture@example.invalid")
	if err != nil {
		t.Fatal(err)
	}
	if err := e.ApplyTracker(context.Background(), p, p.ID, true); err != nil {
		t.Fatal(err)
	}
	text := p.Text + "custom = 'preserve this machine data'\n"
	if err := os.WriteFile(o.Config, []byte(text), 0600); err != nil {
		t.Fatal(err)
	}
	p, err = e.PlanTracker(context.Background(), "", "")
	if err != nil || !p.Exists {
		t.Fatalf("%v %v", p, err)
	}
	if err := e.ApplyTracker(context.Background(), p, p.ID, true); err != nil {
		t.Fatal(err)
	}
	if b, _ := os.ReadFile(o.Config); string(b) != text {
		t.Fatal("extra tracker data lost")
	}
	if err := os.WriteFile(o.Config, []byte("password = secret-no-quotes\n"), 0600); err != nil {
		t.Fatal(err)
	}
	_, err = e.PlanTracker(context.Background(), "", "")
	if err == nil || strings.Contains(err.Error(), "secret-no-quotes") {
		t.Fatal(err)
	}
}
func TestMalformedAdoptionFailsRatherThanCreatingAnotherIdentity(t *testing.T) {
	o := options(t)
	e := local(t, o)
	if err := os.Mkdir(o.State, 0700); err != nil {
		t.Fatal(err)
	}
	b, _ := json.Marshal(map[string]any{"version": 2, "profile": "legacy", "entries": map[string]any{}})
	if err := os.WriteFile(filepath.Join(o.State, "adoption.json"), b, 0600); err != nil {
		t.Fatal(err)
	}
	if _, err := e.SuggestMachine(context.Background()); err == nil {
		t.Fatal("legacy identity guessed")
	}
	if err := e.SaveMachine(context.Background(), "macos_new"); err == nil {
		t.Fatal("legacy approval overwritten")
	}
	if _, err := setup.New(o); err == nil {
		t.Fatal("constructor accepted malformed adoption state")
	}
	after, _ := os.ReadFile(filepath.Join(o.State, "adoption.json"))
	if string(after) != string(b) {
		t.Fatal("malformed approval changed")
	}
}
func TestCustomTrackerInsideHomeIsExcludedFromTrackInventory(t *testing.T) {
	o := options(t)
	o.Config = filepath.Join(o.Home, "private-tracker.toml")
	e := local(t, o)
	if err := e.SaveMachine(context.Background(), "macos_fixture"); err != nil {
		t.Fatal(err)
	}
	p, err := e.PlanTracker(context.Background(), "Fixture", "fixture@example.invalid")
	if err != nil {
		t.Fatal(err)
	}
	if err := e.ApplyTracker(context.Background(), p, p.ID, true); err != nil {
		t.Fatal(err)
	}
	inv, err := e.Policy.InventoryMode(context.Background(), "track")
	if err != nil {
		t.Fatal(err)
	}
	for _, entry := range inv.Entries {
		if entry.Path == "private-tracker.toml" {
			t.Fatal("private tracker offered for enrollment")
		}
	}
}
func TestPendingNativeRecoveryBlocksSourceReplacement(t *testing.T) {
	o := options(t)
	e := local(t, o)
	if err := e.SaveMachine(context.Background(), "macos_fixture"); err != nil {
		t.Fatal(err)
	}
	p, err := e.PlanBackup()
	if err != nil {
		t.Fatal(err)
	}
	b, _ := json.Marshal(native.Journal{Plan: native.Plan{ID: "interrupted"}, Actions: []native.Action{}})
	if err := os.WriteFile(filepath.Join(o.State, "recovery.json"), b, 0600); err != nil {
		t.Fatal(err)
	}
	if err := e.Replace(p, p.Backup); err == nil {
		t.Fatal("replacement abandoned pending native recovery")
	}
	if _, err := e.PlanBackup(); err == nil {
		t.Fatal("planned replacement during recovery")
	}
	if _, err := os.Stat(o.Source); err != nil {
		t.Fatal("recovery source moved")
	}
}
func TestFailedContractCloneIsRetainedAndNeverReportedAsNewOrComplete(t *testing.T) {
	seedOptions := options(t)
	seed := local(t, seedOptions)
	if err := os.Remove(filepath.Join(seed.Options().Source, "dot-contract.json")); err != nil {
		t.Fatal(err)
	}
	git(t, seedOptions.Source, "add", ".")
	git(t, seedOptions.Source, "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "-m", "invalid contract fixture")
	o := options(t)
	url := "https://example.invalid/missing-contract.git"
	config := filepath.Join(filepath.Dir(o.State), "gitconfig")
	text := "[url \"" + seedOptions.Source + "\"]\n insteadOf = " + url + "\n[protocol \"file\"]\n allow = always\n"
	if err := os.WriteFile(config, []byte(text), 0600); err != nil {
		t.Fatal(err)
	}
	t.Setenv("GIT_CONFIG_GLOBAL", config)
	t.Setenv("GIT_CONFIG_NOSYSTEM", "1")
	e, err := setup.New(o)
	if err != nil {
		t.Fatal(err)
	}
	if err := e.CreateSource(context.Background(), "clone", url); err == nil {
		t.Fatal("invalid clone succeeded")
	}
	if _, err := os.Stat(filepath.Join(o.Source, ".git/HEAD")); err != nil {
		t.Fatal("failed checkout deleted")
	}
	if _, err := e.Check(context.Background()); err == nil {
		t.Fatal("invalid clone classified as valid/new")
	}
	state, err := e.InitSource(context.Background(), map[string]any{"action": "detect"})
	if err != nil || !state.Exists || state.Valid {
		t.Fatalf("%v %v", state, err)
	}
	if _, err := os.Stat(o.State); !os.IsNotExist(err) {
		t.Fatal("failed clone created state")
	}
	if err := e.Complete(context.Background()); err == nil {
		t.Fatal("invalid clone complete")
	}
}

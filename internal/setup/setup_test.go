package setup_test

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"strings"
	"testing"

	tea "charm.land/bubbletea/v2"
	"github.com/ujjwal6792/dots-cli/internal/native"
	"github.com/ujjwal6792/dots-cli/internal/setup"
	"github.com/ujjwal6792/dots-cli/internal/tui"
)

func options(t *testing.T) setup.Options {
	t.Helper()
	root, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	home := filepath.Join(root, "home")
	if err := os.Mkdir(home, 0700); err != nil {
		t.Fatal(err)
	}
	return setup.Options{Home: home, Source: filepath.Join(home, ".local/share/dot-config"), State: filepath.Join(root, "isolated-state"), Config: filepath.Join(root, "tracker/chezmoi.toml"), OS: "darwin", Git: "/usr/bin/git"}
}
func TestMain(m *testing.M) {
	// An actual child process tests the terminal boundary, not installation policy.
	// It deliberately installs nothing so its zero exit must NOT imply readiness.
	if os.Getenv("DOT_NATIVE_STREAM_TEST") == "1" && len(os.Args) > 1 && os.Args[1] == "install" {
		b, err := io.ReadAll(os.Stdin)
		if err != nil {
			os.Exit(2)
		}
		_, _ = fmt.Fprint(os.Stdout, "native-out:"+string(b))
		_, _ = fmt.Fprint(os.Stderr, "native-err")
		os.Exit(0)
	}
	os.Exit(m.Run())
}
func TestNativeDependencyChildReceivesStreamsButZeroExitIsNotFalseSuccess(t *testing.T) {
	if os.Geteuid() == 0 {
		t.Skip("brew is intentionally prohibited as root")
	}
	o := options(t)
	o.Git = filepath.Join(o.Home, "missing-git")
	o.Chezmoi = filepath.Join(o.Home, "missing-chezmoi")
	exe, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	o.Brew = exe
	t.Setenv("DOT_NATIVE_STREAM_TEST", "1")
	e, err := setup.New(o)
	if err != nil {
		t.Fatal(err)
	}
	p, err := e.InitDependencies(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	var out, stderr bytes.Buffer
	err = e.InstallDependencies(context.Background(), p.Plan, p.Plan["id"].(string), tui.TerminalStreams{In: strings.NewReader("terminal-answer"), Out: &out, Err: &stderr})
	if err == nil || !strings.Contains(err.Error(), "still missing") {
		t.Fatalf("zero child exit falsely completed dependency installation: %v", err)
	}
	if out.String() != "native-out:terminal-answer" || stderr.String() != "native-err" {
		t.Fatalf("streams not forwarded: %q %q", out.String(), stderr.String())
	}
}
func TestPackageEditingNeverInstallsAndUnavailableStatusIsNotMissing(t *testing.T) {
	o := options(t)
	o.Brew = "/usr/bin/false"
	e, err := setup.New(o)
	if err != nil {
		t.Fatal(err)
	}
	if err := e.CreateSource(context.Background(), "new", "https://example.invalid/dots.git"); err != nil {
		t.Fatal(err)
	}
	s, err := e.Packages(context.Background(), "status", nil, nil, "")
	if err != nil {
		t.Fatal(err)
	}
	digest := s.Sources["brew"].Digest
	_, err = e.Packages(context.Background(), "save", map[string]any{"manager": "brew", "digest": digest, "text": "brew \"git\"\n# unsupported private Ruby preserved\nsystem(\"not-executed\")\n"}, nil, "")
	if err != nil {
		t.Fatal(err)
	}
	s, err = e.Packages(context.Background(), "status", nil, nil, "")
	if err != nil {
		t.Fatal(err)
	}
	if len(s.Rows) != 1 || s.Rows[0].Missing || s.Rows[0].Status != "unavailable" {
		t.Fatal(s)
	}
	if len(s.Sources["brew"].Diagnostics) != 1 {
		t.Fatal(s.Sources)
	}
	if _, err = e.Packages(context.Background(), "plan", nil, nil, ""); err == nil {
		t.Fatal("unavailable/unsupported source installed")
	}
	if _, err = e.Packages(context.Background(), "apply", nil, map[string]any{"id": "unreviewed"}, "unreviewed"); err == nil {
		t.Fatal("installation without terminal succeeded")
	}
}
func step(m tea.Model, k string) tea.Model {
	code := rune(k[0])
	if k == "enter" {
		code = tea.KeyEnter
	}
	if k == "esc" {
		code = tea.KeyEsc
	}
	if k == "space" {
		code = tea.KeySpace
	}
	next, cmd := m.Update(tea.KeyPressMsg{Code: code})
	for i := 0; cmd != nil && i < 12; i++ {
		next, cmd = next.Update(cmd())
	}
	return next
}
func atDone(m tea.Model) bool {
	first, _, _ := strings.Cut(m.View().Content, "\n")
	return strings.TrimSpace(first) == "Done"
}
func TestRealWelcomeSourceMachineDependenciesTrackerDoneJourney(t *testing.T) {
	o := options(t)
	o.User = "fixture"
	o.Chezmoi = "/opt/homebrew/bin/chezmoi"
	if _, err := os.Stat(o.Chezmoi); err != nil {
		t.Skip("real chezmoi needed for completed journey")
	}
	ownedLink, err := os.Readlink(o.Chezmoi)
	if err != nil {
		t.Fatal(err)
	}
	ownedBytes, err := os.ReadFile(o.Chezmoi)
	if err != nil {
		t.Fatal(err)
	}
	e, err := setup.New(o)
	if err != nil {
		t.Fatal(err)
	}
	m := tui.NewInitModel(e)
	if m.Init() != nil {
		t.Fatal("entering setup performed work")
	}
	m = step(m, "enter")
	m = step(m, "j")
	m = step(m, "enter")
	m, _ = m.Update(tea.PasteMsg{Content: "https://example.invalid/owner/dots.git"})
	m = step(m, "enter")
	if !strings.Contains(m.View().Content, "Machine identity") {
		t.Fatal(m.View().Content)
	}
	git(t, o.Source, "config", "user.name", "Fixture Name")
	git(t, o.Source, "config", "user.email", "fixture@example.invalid")
	m = step(m, "enter")
	if !strings.Contains(m.View().Content, "Dependencies") {
		t.Fatal("machine skipped dependencies: " + m.View().Content)
	}
	m = step(m, "enter")
	if !strings.Contains(m.View().Content, "Tracker") {
		t.Fatal(m.View().Content)
	}
	if _, err := os.Stat(o.Config); !os.IsNotExist(err) {
		t.Fatal("tracker review wrote config")
	}
	m = step(m, "enter") // cancel-default
	if _, err := os.Stat(o.Config); !os.IsNotExist(err) {
		t.Fatal("default consent wrote config")
	}
	m = step(m, "enter")
	m = step(m, "j")
	m = step(m, "enter")
	if !atDone(m) || !strings.Contains(m.View().Content, "No files adopted") {
		t.Fatal("false/incomplete Done: " + m.View().Content)
	}
	if _, err := e.ReadConnection(); err != nil {
		t.Fatal(err)
	}
	if err := e.Complete(context.Background()); err != nil {
		t.Fatal(err)
	}
	afterLink, err := os.Readlink(o.Chezmoi)
	if err != nil {
		t.Fatal(err)
	}
	afterBytes, err := os.ReadFile(o.Chezmoi)
	if err != nil {
		t.Fatal(err)
	}
	if ownedLink != afterLink || !bytes.Equal(ownedBytes, afterBytes) {
		t.Fatal("setup changed Homebrew-owned chezmoi")
	}
	m = step(m, "enter")
	if !strings.Contains(m.View().Content, "returning-user actions") {
		t.Fatal("no returning menu: " + m.View().Content)
	}
}
func TestDependenciesListOnlyAllowedToolsAndDoNotInstallDuringInspection(t *testing.T) {
	o := options(t)
	o.Git = filepath.Join(o.Home, "missing-git")
	o.Chezmoi = filepath.Join(o.Home, "missing-chezmoi")
	o.Brew = "/usr/bin/false"
	e, err := setup.New(o)
	if err != nil {
		t.Fatal(err)
	}
	result, err := e.InitDependencies(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if result.Ready || len(result.Rows) != 2 {
		t.Fatal(result)
	}
	if result.Rows[0].Tool != "git" || result.Rows[1].Tool != "chezmoi" {
		t.Fatal(result.Rows)
	}
	if result.Rows[0].Command != "/usr/bin/false install --formula -- git chezmoi" {
		t.Fatal(result.Rows[0])
	}
	for _, r := range result.Rows {
		if !strings.HasPrefix(r.Link, "https://") {
			t.Fatal(r)
		}
	}
	if _, err := os.Stat(o.State); !os.IsNotExist(err) {
		t.Fatal("inspection created state")
	}
	if err := e.InstallDependencies(context.Background(), result.Plan, "", tui.TerminalStreams{}); err == nil {
		t.Fatal("installation without approval/streams succeeded")
	}
}
func TestTrackerRequiresExactConsentAndDetectsRealConfigWithoutChangingGitOrMachine(t *testing.T) {
	o := options(t)
	e, err := setup.New(o)
	if err != nil {
		t.Fatal(err)
	}
	if err := e.CreateSource(context.Background(), "new", "https://example.invalid/dots.git"); err != nil {
		t.Fatal(err)
	}
	if err := e.SaveMachine(context.Background(), "macos_fixture"); err != nil {
		t.Fatal(err)
	}
	git(t, o.Source, "config", "user.name", "Git Name")
	git(t, o.Source, "config", "user.email", "git@example.invalid")
	a, _ := os.ReadFile(filepath.Join(o.State, "adoption.json"))
	g, _ := os.ReadFile(filepath.Join(o.Source, ".git/config"))
	p, err := e.PlanTracker(context.Background(), "Override", "override@example.invalid")
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(p.Text, "Git Name") || strings.Contains(p.Text, "Override") {
		t.Fatal(p.Text)
	}
	if err := e.ApplyTracker(context.Background(), p, p.ID, false); err == nil {
		t.Fatal("missing consent accepted")
	}
	if _, err := os.Stat(o.Config); !os.IsNotExist(err) {
		t.Fatal("review/cancel created tracker")
	}
	if err := e.ApplyTracker(context.Background(), p, "wrong", true); err == nil {
		t.Fatal("wrong plan accepted")
	}
	if err := e.ApplyTracker(context.Background(), p, p.ID, true); err != nil {
		t.Fatal(err)
	}
	before, err := os.ReadFile(o.Config)
	if err != nil {
		t.Fatal(err)
	}
	if info, err := os.Stat(o.Config); err != nil || info.Mode().Perm() != 0600 {
		t.Fatal("tracker permissions")
	}
	p, err = e.PlanTracker(context.Background(), "", "")
	if err != nil || !p.Exists {
		t.Fatalf("tracker not detected: %v %v", p, err)
	}
	after, _ := os.ReadFile(o.Config)
	if string(after) != string(before) {
		t.Fatal("detection rewrote tracker")
	}
	aa, _ := os.ReadFile(filepath.Join(o.State, "adoption.json"))
	gg, _ := os.ReadFile(filepath.Join(o.Source, ".git/config"))
	if string(a) != string(aa) || string(g) != string(gg) {
		t.Fatal("tracker modified Git or machine")
	}
	c, err := e.ReadConnection()
	if err != nil || c.Source != o.Source {
		t.Fatalf("%v %v", c, err)
	}
	reopened, err := setup.New(setup.Options{Home: o.Home, State: o.State, OS: o.OS, Git: o.Git})
	if err != nil {
		t.Fatal(err)
	}
	if reopened.Options().Source != o.Source || reopened.Options().Config != o.Config {
		t.Fatal("connection paths not resolved")
	}
}
func TestCloneHTTPSAndSSHWithRealGitAndVerifiedOrigin(t *testing.T) {
	for _, u := range []string{"https://example.invalid/owner/dots.git", "git@example.invalid:owner/dots.git", "ssh://git@example.invalid/owner/dots.git"} {
		t.Run(u, func(t *testing.T) {
			original := options(t)
			seed, err := setup.New(original)
			if err != nil {
				t.Fatal(err)
			}
			if err := seed.CreateSource(context.Background(), "new", u); err != nil {
				t.Fatal(err)
			}
			git(t, original.Source, "add", ".")
			git(t, original.Source, "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "-m", "fixture contract")
			target := options(t)
			config := filepath.Join(filepath.Dir(target.State), "gitconfig")
			// Real system Git rewrites only transport; configured origin remains HTTPS/SSH.
			text := "[url \"" + original.Source + "\"]\n\tinsteadOf = " + u + "\n[protocol \"file\"]\n\tallow = always\n"
			if err := os.WriteFile(config, []byte(text), 0600); err != nil {
				t.Fatal(err)
			}
			t.Setenv("GIT_CONFIG_GLOBAL", config)
			t.Setenv("GIT_CONFIG_NOSYSTEM", "1")
			e, err := setup.New(target)
			if err != nil {
				t.Fatal(err)
			}
			if strings.HasPrefix(u, "ssh://") {
				var out, stderr bytes.Buffer
				err = e.CreateSourceTerminal(context.Background(), "clone", u, tui.TerminalStreams{In: strings.NewReader(""), Out: &out, Err: &stderr})
				if err != nil {
					t.Fatal(err)
				}
				if !strings.Contains(stderr.String(), "Cloning into") {
					t.Fatal("Git clone did not receive terminal stderr: " + stderr.String())
				}
			} else if err := e.CreateSource(context.Background(), "clone", u); err != nil {
				t.Fatal(err)
			}
			if got := strings.TrimSpace(git(t, target.Source, "config", "--get", "remote.origin.url")); got != u {
				t.Fatal(got)
			}
			if v, err := e.Check(context.Background()); err != nil || v != "old-user" {
				t.Fatalf("%s %v", v, err)
			}
		})
	}
}
func TestSecretURLsAreRejectedBeforeAnyWrites(t *testing.T) {
	for _, u := range []string{"https://secret@example.invalid/r", "https://user:secret@example.invalid/r", "ssh://git:secret@example.invalid/r", "https://example.invalid/r?secret", "https://example.invalid/r#secret", "git@example.invalid:r?secret", "file:///secret", "https://[secret/r"} {
		o := options(t)
		e, err := setup.New(o)
		if err != nil {
			t.Fatal(err)
		}
		err = e.CreateSource(context.Background(), "clone", u)
		if err == nil || strings.Contains(err.Error(), "secret") {
			t.Fatalf("unsafe error: %v", err)
		}
		if _, err := os.Stat(filepath.Dir(o.Source)); !os.IsNotExist(err) {
			t.Fatal("rejected URL wrote parent")
		}
	}
}
func TestReplacePreservesGitDirtyAndUntrackedAndRejectsStalenessCollision(t *testing.T) {
	o := options(t)
	e, err := setup.New(o)
	if err != nil {
		t.Fatal(err)
	}
	if err := e.CreateSource(context.Background(), "new", "https://example.invalid/dots.git"); err != nil {
		t.Fatal(err)
	}
	git(t, o.Source, "add", ".")
	git(t, o.Source, "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "-m", "fixture")
	p, err := e.PlanBackup()
	if err != nil {
		t.Fatal(err)
	}
	if err := e.Replace(p, "wrong"); err == nil {
		t.Fatal("unnamed confirmation accepted")
	}
	dirty := filepath.Join(o.Source, "manifest.json")
	f, err := os.OpenFile(dirty, os.O_APPEND|os.O_WRONLY, 0600)
	if err != nil {
		t.Fatal(err)
	}
	_, _ = f.WriteString("\n")
	_ = f.Close()
	if err := os.WriteFile(filepath.Join(o.Source, "untracked"), []byte("keep me"), 0600); err != nil {
		t.Fatal(err)
	}
	if err := e.Replace(p, p.Backup); err == nil {
		t.Fatal("stale plan accepted")
	}
	p, err = e.PlanBackup()
	if err != nil {
		t.Fatal(err)
	}
	if err := os.Mkdir(filepath.Dir(p.Backup), 0700); err != nil {
		t.Fatal(err)
	}
	if err := e.Replace(p, p.Backup); err == nil {
		t.Fatal("collision accepted")
	}
	before := git(t, o.Source, "status", "--porcelain")
	p, err = e.PlanBackup()
	if err != nil {
		t.Fatal(err)
	}
	if err := e.Replace(p, p.Backup); err != nil {
		t.Fatal(err)
	}
	if after := git(t, p.Backup, "status", "--porcelain"); after != before {
		t.Fatalf("dirty state lost: %q %q", before, after)
	}
	if b, err := os.ReadFile(filepath.Join(p.Backup, "untracked")); err != nil || string(b) != "keep me" {
		t.Fatal("untracked lost")
	}
	if _, err := os.Stat(o.Source); !os.IsNotExist(err) {
		t.Fatal("original not moved")
	}
}
func TestIdentityIsEditableUniqueAndDoesNotEnrollOrCreateTracker(t *testing.T) {
	o := options(t)
	o.User = "Ace"
	e, err := setup.New(o)
	if err != nil {
		t.Fatal(err)
	}
	if err := e.CreateSource(context.Background(), "new", "https://example.invalid/owner/dots.git"); err != nil {
		t.Fatal(err)
	}
	b, err := os.ReadFile(filepath.Join(o.Source, "manifest.json"))
	if err != nil {
		t.Fatal(err)
	}
	var m native.Manifest
	if err := json.Unmarshal(b, &m); err != nil {
		t.Fatal(err)
	}
	m.Machines = map[string]native.MachineInfo{"macos_ace": {OS: "darwin", Label: "Other machine"}}
	b, _ = json.Marshal(m)
	if err := os.WriteFile(filepath.Join(o.Source, "manifest.json"), b, 0600); err != nil {
		t.Fatal(err)
	}
	suggestion, err := e.SuggestMachine(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if !regexp.MustCompile(`^macos_ace_[a-z0-9]{2}$`).MatchString(suggestion.ID) {
		t.Fatal(suggestion)
	}
	if err := e.SaveMachine(context.Background(), "macos_ace"); err == nil {
		t.Fatal("edited collision accepted")
	}
	if err := e.SaveMachine(context.Background(), "CUSTOM_Ace"); err != nil {
		t.Fatal(err)
	}
	a, err := e.Policy.LoadAdoption()
	if err != nil {
		t.Fatal(err)
	}
	if a.Machine.ID != "custom_ace" || a.Version != 3 || len(a.Entries) != 0 {
		t.Fatal(a)
	}
	suggestion, err = e.SuggestMachine(context.Background())
	if err != nil || suggestion != a.Machine {
		t.Fatalf("existing identity not preserved: %v %v", suggestion, err)
	}
	if err := e.SaveMachine(context.Background(), "different"); err == nil {
		t.Fatal("existing identity replaced")
	}
	if _, err := os.Stat(o.Config); !os.IsNotExist(err) {
		t.Fatal("machine created tracker")
	}
	if _, err := os.Stat(filepath.Join(o.State, "connection.json")); !os.IsNotExist(err) {
		t.Fatal("machine connected without consent")
	}
	after, err := os.ReadFile(filepath.Join(o.Source, "manifest.json"))
	if err != nil {
		t.Fatal(err)
	}
	if string(after) != string(b) {
		t.Fatal("identity changed shared machine catalog")
	}
}
func git(t *testing.T, dir string, args ...string) string {
	t.Helper()
	c := exec.Command("/usr/bin/git", append([]string{"-C", dir}, args...)...)
	b, e := c.CombinedOutput()
	if e != nil {
		t.Fatalf("git: %v %s", e, b)
	}
	return string(b)
}
func TestLocalSetupCheckAndContinueAreOfflineAndReadOnly(t *testing.T) {
	o := options(t)
	e, err := setup.New(o)
	if err != nil {
		t.Fatal(err)
	}
	verdict, err := e.Check(context.Background())
	if err != nil || verdict != "new-user" {
		t.Fatalf("%s %v", verdict, err)
	}
	if _, err := os.Stat(o.State); !os.IsNotExist(err) {
		t.Fatal("check created state")
	}
	if err := e.CreateSource(context.Background(), "new", "git@example.invalid:owner/dots.git"); err != nil {
		t.Fatal(err)
	}
	verdict, err = e.Check(context.Background())
	if err != nil || verdict != "old-user" {
		t.Fatalf("%s %v", verdict, err)
	}
	if err := e.Continue(context.Background()); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(o.State); !os.IsNotExist(err) {
		t.Fatal("source setup enrolled a machine or connection")
	}
	if got := git(t, o.Source, "rev-list", "--all"); got != "" {
		t.Fatal("setup committed")
	}
}

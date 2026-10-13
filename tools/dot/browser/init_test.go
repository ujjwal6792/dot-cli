package browser

import (
	"context"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"

	tea "charm.land/bubbletea/v2"
)

type sourceFixture struct {
	fixtureEngine
	exists bool
	calls  []string
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

func TestInitNewUserThroughRealEngineAndURLInput(t *testing.T) {
	engine, err := filepath.Abs("..")
	if err != nil {
		t.Fatal(err)
	}
	home, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	source := filepath.Join(home, "source")
	c := Client{Executable: filepath.Join(engine, "browser_api.py"), Engine: engine, Source: source, Home: home}
	m := sendAndRun(NewInitModel(c), "enter")
	m = send(m, "j")
	m = send(m, "enter")
	url := "ssh://git@example.invalid/owner/dots.git"
	for _, r := range url {
		m, _ = m.Update(tea.KeyPressMsg(tea.Key{Code: r, Text: string(r)}))
	}
	if strings.Contains(m.View().Content, url) {
		t.Fatal("URL exposed in view")
	}
	m = sendAndRun(m, "enter")
	if !strings.Contains(m.View().Content, "Source connected and contract-checked") {
		t.Fatal(m.View().Content)
	}
	got, err := exec.Command("git", "-C", source, "config", "--get", "remote.origin.url").Output()
	if err != nil || strings.TrimSpace(string(got)) != url {
		t.Fatalf("origin %q: %v", got, err)
	}
	if _, err := os.Stat(filepath.Join(home, ".config", "chezmoi", "chezmoi.toml")); !os.IsNotExist(err) {
		t.Fatal("source flow created a tracker")
	}
}

func TestInitRejectsPastedCredentialsWithoutDisplayingOrCreatingSource(t *testing.T) {
	engine, _ := filepath.Abs("..")
	home, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	source := filepath.Join(home, "source")
	c := Client{Executable: filepath.Join(engine, "browser_api.py"), Engine: engine, Source: source, Home: home}
	m := sendAndRun(NewInitModel(c), "enter")
	m = send(m, "enter")
	m, _ = m.Update(tea.PasteMsg{Content: "https://user:super-secret@example.invalid/repo"})
	if strings.Contains(m.View().Content, "super-secret") {
		t.Fatal("pasted secret displayed")
	}
	m = sendAndRun(m, "enter")
	view := m.View().Content
	if !strings.Contains(view, "must not contain credentials") || strings.Contains(view, "super-secret") {
		t.Fatal(view)
	}
	if _, err := os.Stat(source); !os.IsNotExist(err) {
		t.Fatal("credential rejection created a source")
	}
}

func TestInitExistingUserPastesHTTPSAndSSHThroughRealEngine(t *testing.T) {
	for _, url := range []string{"https://example.invalid/owner/dots.git", "git@example.invalid:owner/dots.git"} {
		t.Run(url, func(t *testing.T) {
			engine, _ := filepath.Abs("..")
			home, err := filepath.EvalSymlinks(t.TempDir())
			if err != nil {
				t.Fatal(err)
			}
			t.Setenv("GIT_CONFIG_GLOBAL", filepath.Join(home, "gitconfig"))
			t.Setenv("GIT_CONFIG_NOSYSTEM", "1")
			remote := filepath.Join(home, "remote")
			if err := os.MkdirAll(filepath.Join(remote, "home"), 0700); err != nil {
				t.Fatal(err)
			}
			files := map[string]string{
				"dot-contract.json": `{"protocol":1,"engine_api":{"min":1,"max":1},"manifest_version":1,"approval_version":2,"manifest":"manifest.json","home_source":"home","required_adapters":["intent","scope","gitconfig","skills","layout"]}`,
				"manifest.json":     `{"version":1,"entries":{},"profiles":{}}`,
				"home/dot_fixture":  "original\n",
			}
			for name, content := range files {
				if err := os.WriteFile(filepath.Join(remote, name), []byte(content), 0600); err != nil {
					t.Fatal(err)
				}
			}
			git := func(args ...string) string {
				t.Helper()
				got, err := exec.Command("git", args...).CombinedOutput()
				if err != nil {
					t.Fatalf("git %v: %s: %v", args, got, err)
				}
				return strings.TrimSpace(string(got))
			}
			git("init", remote)
			git("-C", remote, "add", ".")
			git("-C", remote, "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "-m", "fixture")
			git("config", "--global", "url."+remote+".insteadOf", url)
			source := filepath.Join(home, "source")
			c := Client{Executable: filepath.Join(engine, "browser_api.py"), Engine: engine, Source: source, Home: home}
			m := sendAndRun(NewInitModel(c), "enter")
			m = send(m, "enter")
			m, _ = m.Update(tea.PasteMsg{Content: url})
			m = sendAndRun(m, "enter")
			if !strings.Contains(m.View().Content, "Source connected and contract-checked") {
				t.Fatal(m.View().Content)
			}
			if got := git("-C", source, "config", "--get", "remote.origin.url"); got != url {
				t.Fatal(got)
			}
			if _, err := os.Stat(filepath.Join(source, "home", "dot_fixture")); err != nil {
				t.Fatal(err)
			}
			m = sendAndRun(NewInitModel(c), "enter")
			m = sendAndRun(m, "enter")
			if !strings.Contains(m.View().Content, "Source connected and contract-checked") {
				t.Fatal(m.View().Content)
			}
		})
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

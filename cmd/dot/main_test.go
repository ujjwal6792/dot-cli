package main

import (
	"bytes"
	"context"
	"encoding/json"
	"github.com/ujjwal6792/dots-cli/internal/native"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestHelpBeforePrivateGates(t *testing.T) {
	for _, args := range [][]string{{"--help"}, {"intent", "apply", "--help"}, {"init", "--help"}, {"--version"}, {"uninstall", "--help"}} {
		var out, errOut bytes.Buffer
		if code := run(context.Background(), args, strings.NewReader(""), &out, &errOut); code != 0 || out.Len() == 0 {
			t.Fatalf("%v: code %d, %s", args, code, &errOut)
		}
	}
}

func TestCompleteHelpDocumentsAppAliasesAndRetiredInventory(t *testing.T) {
	t.Setenv("HOME", "malformed")
	for _, args := range [][]string{{"--help"}, {"intent", "inventory", "--help"}} {
		var out, errs bytes.Buffer
		if code := run(context.Background(), args, strings.NewReader(""), &out, &errs); code != 0 {
			t.Fatalf("help failed: %d %s", code, &errs)
		}
		for _, want := range []string{"Bare dot", "read-only", "--format json", "intent rounds", "Retired:", "no checkout fallback"} {
			if !strings.Contains(out.String(), want) {
				t.Fatalf("%v help omits %q", args, want)
			}
		}
	}
}

func TestScriptedSetupAndExactIntent(t *testing.T) {
	home := t.TempDir()
	home, _ = filepath.EvalSymlinks(home)
	t.Setenv("HOME", home)
	t.Setenv("XDG_STATE_HOME", "")
	t.Setenv("XDG_CONFIG_HOME", "")
	t.Setenv("DOT_SOURCE", "")
	t.Setenv("DOT_CONFIG", "")
	t.Setenv("DOT_STATE_DIR", "")
	invoke := func(args ...string) string {
		t.Helper()
		var out, errs bytes.Buffer
		if code := run(context.Background(), args, strings.NewReader(""), &out, &errs); code != 0 {
			t.Fatalf("%v: %d %s", args, code, &errs)
		}
		return out.String()
	}
	if !strings.Contains(invoke("init", "--check"), "new-user") {
		t.Fatal("not new")
	}
	if _, err := os.Stat(filepath.Join(home, ".local")); !os.IsNotExist(err) {
		t.Fatal("check wrote directories")
	}
	invoke("init", "--action", "new", "--url", "https://example.com/config.git")
	invoke("init", "--machine", "macos_test")
	os.WriteFile(filepath.Join(home, ".local/share/dot-config/manifest.json"), []byte(`{"version":2,"entries":{},"profiles":{"test":[]}}`), 0600)
	if !strings.Contains(invoke("init", "--check"), "old-user") {
		t.Fatal("not old")
	}
	status := invoke("status", "--format", "json")
	if !strings.Contains(status, "macos_test") {
		t.Fatal(status)
	}
	request := filepath.Join(home, "request.json")
	os.WriteFile(request, []byte(`{"version":1,"mode":"adopt","profile":"test","entries":[]}`), 0600)
	planJSON := invoke("intent", "plan", "--request", request)
	var plan native.Plan
	if err := json.Unmarshal([]byte(planJSON), &plan); err != nil {
		t.Fatal(err)
	}
	file := filepath.Join(home, "plan.json")
	os.WriteFile(file, []byte(planJSON), 0600)
	preview := invoke("intent", "preview", "--plan", file)
	if !strings.Contains(preview, plan.ID) || !strings.Contains(preview, "macos_test") || !strings.Contains(preview, "repository → target") {
		t.Fatal("preview omits exact identity/direction: " + preview)
	}
	var out, errs bytes.Buffer
	if code := run(context.Background(), []string{"intent", "apply", "--plan", file}, strings.NewReader(""), &out, &errs); code == 0 {
		t.Fatal("missing approval accepted")
	}
	invoke("intent", "apply", "--plan", file, "--approve", plan.ID)
	if !strings.Contains(invoke("scope", "show", "--format", "json"), "macos_test") {
		t.Fatal("scope machine missing")
	}
	invoke("diff", "--format", "json")
}

func TestUninstallAliasNeedsOwnerNotPrivateConfig(t *testing.T) {
	t.Setenv("HOME", "not-an-absolute-path")
	for _, args := range [][]string{{"-u"}, {"--uninstall"}, {"uninstall"}} {
		var out, errs bytes.Buffer
		if code := run(context.Background(), args, strings.NewReader(""), &out, &errs); code != 2 || !strings.Contains(errs.String(), "original manager") {
			t.Fatalf("%v: %d %s", args, code, &errs)
		}
	}
}

func TestHelpWordAsFlagValueDoesNotBypassOperation(t *testing.T) {
	var out, errs bytes.Buffer
	code := run(context.Background(), []string{"init", "--home", "relative", "--machine", "help"}, strings.NewReader(""), &out, &errs)
	if code == 0 || out.Len() != 0 {
		t.Fatalf("flag value became help: code=%d out=%s", code, &out)
	}
}

func TestInteractiveCommandsCannotPretendToProduceJSON(t *testing.T) {
	var out, errs bytes.Buffer
	code := run(context.Background(), []string{"browse", "--format", "json", "--home", "relative"}, strings.NewReader(""), &out, &errs)
	if code != 2 || !strings.Contains(errs.String(), "unsupported --format") {
		t.Fatalf("%d %s", code, &errs)
	}
}

func TestRetiredCommandsAndMisplacedConsentNeverSucceed(t *testing.T) {
	for _, args := range [][]string{{"run", "anything"}, {"update", "name", "version"}, {"layout", "plan"}, {"migrate"}, {"status", "--ack-unsafe"}, {"init", "--yes"}, {"init", "--machine", "machine", "--url", "https://example.com/config.git"}} {
		var out, errs bytes.Buffer
		if code := run(context.Background(), args, strings.NewReader(""), &out, &errs); code != 2 {
			t.Fatalf("%v: %d %s", args, code, &errs)
		}
	}
}

func TestHealthDiagnosesDependenciesBeforeMalformedPrivatePaths(t *testing.T) {
	var out, errs bytes.Buffer
	code := run(context.Background(), []string{"--health", "--home", "relative", "--source", "also-relative", "--git", "/nonexistent/dot18-git"}, strings.NewReader(""), &out, &errs)
	if code != 1 || !strings.Contains(errs.String(), "git missing") {
		t.Fatalf("%d %s", code, &errs)
	}
}

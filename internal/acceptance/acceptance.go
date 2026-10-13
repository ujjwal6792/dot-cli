// Package acceptance exercises an extracted release through its executable,
// independently of cwd/repository files. It never installs host dependencies.
package acceptance

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"github.com/ujjwal6792/dots-cli/internal/distribution"
	"github.com/ujjwal6792/dots-cli/internal/native"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
)

type harness struct {
	ctx                         context.Context
	dot, git, dir, path, global string
}

func (h harness) env(home string) []string {
	return []string{"HOME=" + home, "PATH=" + h.path, "LC_ALL=C", "TERM=xterm-256color", "GIT_CONFIG_GLOBAL=" + h.global, "GIT_CONFIG_NOSYSTEM=1", "GIT_TERMINAL_PROMPT=0"}
}
func (h harness) invoke(home, input string, args ...string) ([]byte, error) {
	c := exec.CommandContext(h.ctx, h.dot, args...)
	c.Dir = h.dir
	c.Env = h.env(home)
	c.Stdin = strings.NewReader(input)
	var out, errs bytes.Buffer
	c.Stdout = &out
	c.Stderr = &errs
	if err := c.Run(); err != nil {
		return nil, fmt.Errorf("dot %v failed: %w: %s", args, err, &errs)
	}
	return out.Bytes(), nil
}
func (h harness) gitRun(home string, args ...string) error {
	c := exec.CommandContext(h.ctx, h.git, args...)
	c.Dir = h.dir
	c.Env = h.env(home)
	b, err := c.CombinedOutput()
	if err != nil {
		return fmt.Errorf("Git %v: %w: %s", args, err, b)
	}
	return nil
}
func write(path string, b []byte, mode os.FileMode) error {
	if err := os.MkdirAll(filepath.Dir(path), 0700); err != nil {
		return err
	}
	return os.WriteFile(path, b, mode)
}
func marshal(v any) string { b, _ := json.Marshal(v); return string(b) }
func (h harness) plan(home string, r native.Request) (native.Plan, error) {
	var p native.Plan
	b, err := h.invoke(home, marshal(r), "intent", "plan", "--request", "-")
	if err != nil {
		return p, err
	}
	err = native.Decode(b, &p)
	return p, err
}
func (h harness) apply(home string, p native.Plan) error {
	if _, err := h.invoke(home, marshal(p), "intent", "preview", "--plan", "-"); err != nil {
		return err
	}
	if _, err := h.invoke(home, marshal(p), "intent", "apply", "--plan", "-", "--approve", "wrong"); err == nil {
		return fmt.Errorf("wrong approval was accepted")
	}
	args := []string{"intent", "apply", "--plan", "-", "--approve", p.ID}
	if p.Gates.Unsafe {
		args = append(args, "--ack-unsafe")
	}
	if p.Gates.Desktop {
		args = append(args, "--ack-desktop")
	}
	if p.Gates.Backup {
		args = append(args, "--ack-backup")
	}
	_, err := h.invoke(home, marshal(p), args...)
	return err
}
func (h harness) tracker(home, id string) error {
	if _, err := h.invoke(home, "", "init", "--machine", id); err != nil {
		return err
	}
	b, err := h.invoke(home, "", "init", "--tracker-plan", "--name", "Native Acceptance", "--email", "native@example.invalid")
	if err != nil {
		return err
	}
	var p struct {
		ID string `json:"id"`
	}
	if err := json.Unmarshal(b, &p); err != nil {
		return err
	}
	if _, err := h.invoke(home, string(b), "init", "--tracker-apply", "-", "--approve", p.ID, "--yes"); err != nil {
		return err
	}
	_, err = h.invoke(home, "", "init", "--complete")
	return err
}

// Run refuses cross-platform execution. Linux execution is owner-waived, not
// inferred from cross-builds. The caller supplies a NEW absolute fixture root.
func Run(ctx context.Context, archive, checksum, dir string) error {
	var err error
	archive, err = filepath.Abs(archive)
	if err != nil {
		return err
	}
	a, err := distribution.Verify(archive, checksum)
	if err != nil {
		return err
	}
	if a.Manifest.Platform != distribution.Platform() {
		return fmt.Errorf("cannot execute %s on %s; cross-build verification is not runtime acceptance", a.Manifest.Platform, distribution.Platform())
	}
	if !filepath.IsAbs(dir) || filepath.Clean(dir) != dir {
		return fmt.Errorf("absolute new acceptance directory required")
	}
	if err := os.Mkdir(dir, 0700); err != nil {
		return err
	}
	if _, err := distribution.Package("999.0.0", a.Manifest.Platform, a.Files["dot"], a.Files["libexec/chezmoi"], a.Files["licenses/chezmoi.txt"]); err == nil {
		return fmt.Errorf("forged release version accepted")
	}
	one, err := distribution.Package(a.Manifest.Version, a.Manifest.Platform, a.Files["dot"], a.Files["libexec/chezmoi"], a.Files["licenses/chezmoi.txt"])
	if err != nil {
		return err
	}
	again, err := distribution.Package(a.Manifest.Version, a.Manifest.Platform, a.Files["dot"], a.Files["libexec/chezmoi"], a.Files["licenses/chezmoi.txt"])
	if err != nil || !bytes.Equal(one, again) || distribution.Hash(one) != checksum {
		return fmt.Errorf("package contents are not reproducible")
	}
	bundle := filepath.Join(dir, "extracted")
	if err := distribution.Extract(a, bundle); err != nil {
		return err
	}
	git, err := exec.LookPath("git")
	if err != nil {
		return err
	}
	git, err = filepath.Abs(git)
	if err != nil {
		return err
	}
	path := filepath.Join(dir, "path")
	if err := os.Mkdir(path, 0700); err != nil {
		return err
	}
	if err := os.Symlink(git, filepath.Join(path, "git")); err != nil {
		return err
	}
	h := harness{ctx, filepath.Join(bundle, "dot"), git, dir, path, filepath.Join(dir, "gitconfig")}
	home := filepath.Join(dir, "home-one")
	if err := os.Mkdir(home, 0700); err != nil {
		return err
	}
	// PATH contains ONLY system Git. No shell, interpreter, gum, gh, Go compiler,
	// package manager or system chezmoi is available to dot.
	for _, args := range [][]string{{"--help"}, {"intent", "apply", "--help"}, {"--version"}, {"--health"}, {"uninstall", "--help"}} {
		if _, err := h.invoke(home, "", args...); err != nil {
			return err
		}
	}
	if err := h.openApp(home, "Welcome"); err != nil {
		return err
	}
	if err := h.openApp(home, "Welcome", "init"); err != nil {
		return err
	}
	b, err := h.invoke(home, "", "init", "--check")
	if err != nil || !bytes.Contains(b, []byte("new-user")) {
		return fmt.Errorf("new-user check: %s %v", b, err)
	}
	if _, err := os.Stat(filepath.Join(home, ".local")); !os.IsNotExist(err) {
		return fmt.Errorf("read-only check wrote HOME")
	}
	url := "https://native.example.invalid/config.git"
	if _, err := h.invoke(home, "", "init", "--action", "new", "--url", url); err != nil {
		return err
	}
	id := "macos_one"
	if runtime.GOOS == "linux" {
		id = "linux_one"
	}
	if err := h.tracker(home, id); err != nil {
		return err
	}
	if err := h.openApp(home, "returning-user actions"); err != nil {
		return err
	}
	if err := h.openApp(home, "returning-user actions", "init"); err != nil {
		return err
	}
	source := filepath.Join(home, ".local/share/dot-config")
	b, err = h.invoke(home, "", "init", "--check")
	if err != nil || !bytes.Contains(b, []byte("old-user")) {
		return fmt.Errorf("old-user check: %s %v", b, err)
	}
	if err := write(filepath.Join(home, "prefs"), []byte("first native config\n"), 0755); err != nil {
		return err
	}
	p, err := h.plan(home, native.Request{Version: 1, Mode: "track", Profile: "base", Selections: []native.Selection{{Path: "prefs", Entry: native.Entry{OS: []string{runtime.GOOS}, Portability: "managed-shared", Impact: "Safe"}}}})
	if err != nil {
		return err
	}
	if err := h.apply(home, p); err != nil {
		return err
	}
	adoption, err := os.ReadFile(filepath.Join(home, ".local/state/dot/adoption.json"))
	if err != nil {
		return err
	}
	var adopted native.Adoption
	if err := native.Decode(adoption, &adopted); err != nil {
		return err
	}
	if len(adopted.Entries) != 0 {
		return fmt.Errorf("Track implicitly adopted")
	}
	// Local remote reached through real Git's transport rewrite. Public origin
	// remains the credential-free HTTPS URL; no hosting API/gh is involved.
	remote := filepath.Join(dir, "remote.git")
	if err := h.gitRun(home, "init", "--bare", remote); err != nil {
		return err
	}
	if err := h.gitRun(home, "config", "--file", h.global, "url."+remote+".insteadOf", url); err != nil {
		return err
	}
	if err := h.gitRun(home, "-C", source, "add", "."); err != nil {
		return err
	}
	if err := h.gitRun(home, "-C", source, "-c", "user.name=Native Acceptance", "-c", "user.email=native@example.invalid", "commit", "-m", "reviewed native configuration"); err != nil {
		return err
	}
	if err := h.gitRun(home, "-C", source, "push", "--set-upstream", "origin", "HEAD"); err != nil {
		return err
	}
	if _, err := h.invoke(home, "", "push"); err == nil {
		return fmt.Errorf("push lacked explicit consent")
	}
	if _, err := h.invoke(home, "", "push", "--yes"); err != nil {
		return err
	}
	two := filepath.Join(dir, "home-two")
	if err := os.Mkdir(two, 0700); err != nil {
		return err
	}
	if _, err := h.invoke(two, "", "init", "--action", "clone", "--url", url); err != nil {
		return err
	}
	idTwo := "macos_two"
	if runtime.GOOS == "linux" {
		idTwo = "linux_two"
	}
	if err := h.tracker(two, idTwo); err != nil {
		return err
	}
	if _, err := h.invoke(two, "", "pull", "--yes"); err != nil {
		return err
	}
	if _, err := os.Stat(filepath.Join(two, "prefs")); !os.IsNotExist(err) {
		return fmt.Errorf("pull automatically applied target")
	}
	for _, args := range [][]string{{"status", "--format", "json"}, {"diff", "--format", "json"}, {"scope", "show", "--format", "json"}} {
		b, err := h.invoke(two, "", args...)
		if err != nil {
			return err
		}
		if !bytes.Contains(b, []byte(idTwo)) {
			return fmt.Errorf("machine-scoped JSON missing identity: %v", args)
		}
	}
	p, err = h.plan(two, native.Request{Version: 1, Mode: "adopt", Profile: "base", Entries: []string{"prefs"}})
	if err != nil {
		return err
	}
	if err := h.apply(two, p); err != nil {
		return err
	}
	got, err := os.ReadFile(filepath.Join(two, "prefs"))
	if err != nil || string(got) != "first native config\n" {
		return fmt.Errorf("real packaged adoption bytes: %q %v", got, err)
	}
	st, err := os.Stat(filepath.Join(two, "prefs"))
	if err != nil || st.Mode().Perm() != 0755 {
		return fmt.Errorf("real packaged adoption mode mismatch")
	}
	// Stale reviewed intent must refuse a later human edit.
	p, err = h.plan(two, native.Request{Version: 1, Mode: "update", Profile: "base"})
	if err != nil {
		return err
	}
	if err := os.WriteFile(filepath.Join(two, "prefs"), []byte("human edit\n"), 0640); err != nil {
		return err
	}
	if _, err := h.invoke(two, marshal(p), "intent", "apply", "--plan", "-", "--approve", p.ID); err == nil {
		return fmt.Errorf("stale plan applied")
	}
	p, err = h.plan(two, native.Request{Version: 1, Mode: "update", Profile: "base"})
	if err != nil {
		return err
	}
	if err := h.apply(two, p); err != nil {
		return err
	}
	// Installation ownership, checksum-before-promotion, local self-update and
	// uninstall are isolated from host directories and private machine state.
	root := filepath.Join(dir, "installation")
	if _, err := h.invoke(home, "", "install", "--root", root, "--archive", archive, "--sha256", strings.Repeat("0", 64), "--yes"); err == nil {
		return fmt.Errorf("bad checksum installed")
	}
	if _, err := os.Stat(root); !os.IsNotExist(err) {
		return fmt.Errorf("bad checksum mutated installation root")
	}
	if _, err := h.invoke(home, "", "install", "--root", root, "--archive", archive, "--sha256", checksum, "--yes"); err != nil {
		return err
	}
	installed := h
	installed.dot = filepath.Join(root, "bin/dot")
	if _, err := installed.invoke(two, "", "--health"); err != nil {
		return err
	}
	before, err := os.ReadFile(filepath.Join(two, ".local/state/dot/adoption.json"))
	if err != nil {
		return err
	}
	if _, err := installed.invoke(two, "", "self-update", "--root", root, "--archive", archive, "--sha256", checksum, "--yes"); err != nil {
		return err
	}
	after, err := os.ReadFile(filepath.Join(two, ".local/state/dot/adoption.json"))
	if err != nil || !bytes.Equal(before, after) {
		return fmt.Errorf("public update changed private approval")
	}
	// A durable interrupted promotion journal is an external fixture, not a
	// success flag. The real installed recovery command must validate and clear it.
	target, err := os.Readlink(filepath.Join(root, "current"))
	if err != nil {
		return err
	}
	journal := map[string]any{"schema": 1, "previous": target, "next": target}
	if err := write(filepath.Join(root, "promotion.json"), []byte(marshal(journal)), 0600); err != nil {
		return err
	}
	if _, err := installed.invoke(two, "", "self-update", "--root", root, "--archive", archive, "--sha256", checksum, "--yes"); err == nil {
		return fmt.Errorf("pending promotion did not block update")
	}
	if _, err := installed.invoke(two, "", "install-recover", "--root", root, "--yes"); err != nil {
		return err
	}
	if _, err := installed.invoke(two, "", "uninstall", "--root", root, "--yes"); err != nil {
		return err
	}
	if _, err := os.Lstat(filepath.Join(root, "bin/dot")); !os.IsNotExist(err) {
		return fmt.Errorf("uninstall retained launch link")
	}
	after, err = os.ReadFile(filepath.Join(two, ".local/state/dot/adoption.json"))
	if err != nil || !bytes.Equal(before, after) {
		return fmt.Errorf("uninstall changed private approval")
	}
	fmt.Printf("PASS: real extracted %s setup, tracker, Track, clone, explicit pull/push, machine JSON, exact adoption/staleness, private install/update/recovery/uninstall\nFixtures retained at %s\n", distribution.Platform(), dir)
	return nil
}

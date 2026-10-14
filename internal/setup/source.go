package setup

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"strings"
	"syscall"
	"time"
	"unicode"

	"github.com/ujjwal6792/dots-cli/internal/native"
	"github.com/ujjwal6792/dots-cli/internal/tui"
)

// ValidateURL never echoes untrusted input (including URL parser errors).
func ValidateURL(raw string) error {
	bad := fmt.Errorf("source URL must be credential-free HTTPS or SSH, without query or fragment")
	if raw == "" || strings.ContainsAny(raw, "?#\\") || strings.IndexFunc(raw, func(r rune) bool { return unicode.IsSpace(r) || unicode.IsControl(r) }) >= 0 {
		return bad
	}
	if regexp.MustCompile(`^git@[a-zA-Z0-9][a-zA-Z0-9.-]*:[a-zA-Z0-9_./~-]+$`).MatchString(raw) {
		return nil
	}
	u, err := url.Parse(raw)
	if err != nil || u.Hostname() == "" || u.Path == "" || u.Path == "/" || u.Opaque != "" {
		return bad
	}
	if u.Scheme != "https" && u.Scheme != "ssh" {
		return bad
	}
	if u.User != nil {
		if u.Scheme != "ssh" {
			return bad
		}
		if _, ok := u.User.Password(); ok {
			return bad
		}
		if !regexp.MustCompile(`^[a-zA-Z0-9_-]+$`).MatchString(u.User.Username()) {
			return bad
		}
	}
	if strings.Contains(u.Path, "@") || u.RawQuery != "" || u.Fragment != "" {
		return bad
	}
	return nil
}
func (e *Engine) git(ctx context.Context, args ...string) (string, error) {
	c := e.gitCommand(ctx, args...)
	b, err := c.Output()
	if err != nil {
		return "", fmt.Errorf("source Git operation failed; checkout retained, verify access and retry")
	}
	return strings.TrimSpace(string(b)), nil
}
func (e *Engine) gitCommand(ctx context.Context, args ...string) *exec.Cmd {
	c := exec.CommandContext(ctx, e.options.Git, args...)
	c.Env = append(os.Environ(), "HOME="+e.options.Home, "XDG_CONFIG_HOME="+filepath.Join(e.options.Home, ".config"), "GIT_TERMINAL_PROMPT=0")
	return c
}
func (e *Engine) Continue(ctx context.Context) error {
	if _, err := canonicalPath(e.options.Source); err != nil {
		return err
	}
	if _, err := e.Policy.CheckContract(ctx); err != nil {
		return fmt.Errorf("source contract check failed: %w", err)
	}
	origin, err := e.git(ctx, "-C", e.options.Source, "config", "--get-all", "remote.origin.url")
	if err != nil {
		return err
	}
	if err := ValidateURL(origin); err != nil {
		return fmt.Errorf("source origin is not credential-free HTTPS/SSH; review Git config")
	}
	return nil
}
func (e *Engine) Check(ctx context.Context) (string, error) {
	if _, err := canonicalPath(e.options.Source); err != nil {
		return "", err
	}
	_, err := os.Lstat(e.options.Source)
	if os.IsNotExist(err) {
		return "new-user", nil
	}
	if err != nil {
		return "", err
	}
	if err := e.Continue(ctx); err != nil {
		return "", err
	}
	return "old-user", nil
}
func (e *Engine) CreateSource(ctx context.Context, action, raw string) error {
	return e.createSource(ctx, action, raw, nil)
}

// CreateSourceTerminal delegates Git/SSH/credential prompts to native Git after
// Bubble Tea releases its terminal. The app never reads authentication answers.
func (e *Engine) CreateSourceTerminal(ctx context.Context, action, raw string, s tui.TerminalStreams) error {
	if s.In == nil || s.Out == nil || s.Err == nil {
		return fmt.Errorf("native Git terminal streams required")
	}
	return e.createSource(ctx, action, raw, &s)
}
func (e *Engine) createSource(ctx context.Context, action, raw string, s *tui.TerminalStreams) error {
	if err := ValidateURL(raw); err != nil {
		return err
	} // before any write or Git invocation
	if action != "new" && action != "clone" {
		return fmt.Errorf("unsupported source action")
	}
	if err := ctx.Err(); err != nil {
		return err
	}
	if !e.gitAvailable(ctx) {
		return fmt.Errorf("Git missing/incompatible; review Dependencies before source creation")
	}
	if err := e.noRecovery(); err != nil {
		return err
	}
	p := e.options.Source
	if _, err := canonicalPath(p); err != nil {
		return err
	}
	if _, err := os.Lstat(p); err == nil {
		return fmt.Errorf("source already exists; Continue or confirm Replace first")
	} else if !os.IsNotExist(err) {
		return err
	}
	if err := os.MkdirAll(filepath.Dir(p), 0700); err != nil {
		return err
	}
	if _, err := canonicalPath(p); err != nil {
		return err
	}
	if err := os.Mkdir(p, 0700); err != nil {
		return err
	}
	if action == "clone" {
		if s != nil {
			cmd := e.gitCommand(ctx, "clone", "--", raw, p)
			cmd.Env = append(cmd.Env, "GIT_TERMINAL_PROMPT=1")
			cmd.Stdin = s.In
			cmd.Stdout = s.Out
			cmd.Stderr = s.Err
			if err := cmd.Run(); err != nil {
				return fmt.Errorf("source Git clone failed or interrupted; checkout retained for review")
			}
		} else if _, err := e.git(ctx, "clone", "--", raw, p); err != nil {
			return err
		}
	} else {
		if _, err := e.git(ctx, "init", "--", p); err != nil {
			return err
		}
		if _, err := e.git(ctx, "-C", p, "remote", "add", "origin", raw); err != nil {
			return err
		}
		if err := os.Mkdir(filepath.Join(p, "home"), 0700); err != nil {
			return err
		}
		if err := exclusive(filepath.Join(p, "home/.gitkeep"), nil); err != nil {
			return err
		}
		c := native.Contract{Protocol: 1, EngineAPI: native.VersionRange{Min: 1, Max: 1}, ManifestVersion: 2, ApprovalVersion: 3, Manifest: "manifest.json", HomeSource: "home", RequiredAdapters: []string{"intent", "scope"}}
		if err := exclusive(filepath.Join(p, "dot-contract.json"), encode(c)); err != nil {
			return err
		}
		if err := exclusive(filepath.Join(p, "manifest.json"), encode(native.Manifest{Version: 2, Entries: map[string]native.Entry{}, Profiles: map[string][]string{}})); err != nil {
			return err
		}
	}
	origin, err := e.git(ctx, "-C", p, "config", "--get", "remote.origin.url")
	if err != nil {
		return err
	}
	if origin != raw {
		return fmt.Errorf("source origin verification failed; checkout retained")
	}
	return e.Continue(ctx)
}

type BackupPlan struct {
	Source      string `json:"source"`
	Backup      string `json:"backup"`
	Fingerprint string `json:"fingerprint"`
}

func fingerprint(root string) (string, error) {
	h := sha256.New()
	err := filepath.WalkDir(root, func(p string, d fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if d.Type()&os.ModeSymlink != 0 {
			return fmt.Errorf("source contains symlink; backup rejected")
		}
		i, err := d.Info()
		if err != nil {
			return err
		}
		if !i.IsDir() && !i.Mode().IsRegular() {
			return fmt.Errorf("source contains nonregular data")
		}
		r, _ := filepath.Rel(root, p)
		s := i.Sys().(*syscall.Stat_t)
		fmt.Fprintf(h, "%s\x00%d\x00%d\x00%d\x00%d\x00", r, i.Mode(), s.Dev, s.Ino, i.ModTime().UnixNano())
		if !i.IsDir() {
			file, err := os.Open(p)
			if err != nil {
				return err
			}
			_, err = io.Copy(h, file)
			closeErr := file.Close()
			if err != nil {
				return err
			}
			if closeErr != nil {
				return closeErr
			}
		}
		return nil
	})
	if err != nil {
		return "", err
	}
	return hex.EncodeToString(h.Sum(nil)), nil
}
func (e *Engine) PlanBackup() (BackupPlan, error) {
	if err := e.noRecovery(); err != nil {
		return BackupPlan{}, err
	}
	p := e.options.Source
	if _, err := canonicalPath(p); err != nil {
		return BackupPlan{}, err
	}
	i, err := os.Stat(p)
	if err != nil {
		return BackupPlan{}, err
	}
	if !i.IsDir() {
		return BackupPlan{}, fmt.Errorf("source is not a directory")
	}
	f, err := fingerprint(p)
	if err != nil {
		return BackupPlan{}, err
	}
	backup := p + ".backup-" + time.Now().UTC().Format("20060102T150405.000000000Z") + "/checkout"
	if _, err := os.Lstat(filepath.Dir(backup)); !os.IsNotExist(err) {
		return BackupPlan{}, fmt.Errorf("backup collision")
	}
	return BackupPlan{Source: p, Backup: backup, Fingerprint: f}, nil
}
func (e *Engine) Replace(p BackupPlan, confirmedPath string) error {
	if err := e.noRecovery(); err != nil {
		return err
	}
	if p.Source != e.options.Source || confirmedPath != p.Backup || !regexp.MustCompile("^"+regexp.QuoteMeta(p.Source)+`\.backup-\d{8}T\d{6}\.\d{9}Z/checkout$`).MatchString(p.Backup) {
		return fmt.Errorf("exact named backup confirmation required")
	}
	if _, err := canonicalPath(p.Source); err != nil {
		return err
	}
	if _, err := canonicalPath(p.Backup); err != nil {
		return err
	}
	f, err := fingerprint(p.Source)
	if err != nil {
		return err
	}
	if f != p.Fingerprint {
		return fmt.Errorf("stale backup plan; original retained, review again")
	}
	// Exclusive parent reservation prevents overwrite, even of an empty directory.
	if err := os.Mkdir(filepath.Dir(p.Backup), 0700); err != nil {
		return fmt.Errorf("backup collision/reservation failed: %w", err)
	}
	return os.Rename(p.Source, p.Backup)
}
func (e *Engine) noRecovery() error {
	_, err := e.Policy.Recovery()
	if err == nil {
		return fmt.Errorf("pending native recovery; resolve it before setup or source replacement")
	}
	if !errors.Is(err, os.ErrNotExist) {
		return fmt.Errorf("native recovery record requires review: %w", err)
	}
	return nil
}
func (e *Engine) InitSource(ctx context.Context, r map[string]any) (tui.SourceResult, error) {
	var req struct {
		Action string      `json:"action"`
		URL    string      `json:"url,omitempty"`
		Plan   *BackupPlan `json:"plan,omitempty"`
	}
	b, _ := json.Marshal(r)
	if err := native.Decode(b, &req); err != nil {
		return tui.SourceResult{}, fmt.Errorf("invalid source request")
	}
	switch req.Action {
	case "detect":
		if _, err := canonicalPath(e.options.Source); err != nil {
			return tui.SourceResult{}, err
		}
		if !e.gitAvailable(ctx) {
			return tui.SourceResult{NeedsGit: true}, nil
		}
		_, err := os.Stat(e.options.Source)
		if os.IsNotExist(err) {
			return tui.SourceResult{}, nil
		}
		if err != nil {
			return tui.SourceResult{}, err
		}
		contErr := e.Continue(ctx)
		errMsg := ""
		if contErr != nil {
			errMsg = contErr.Error()
		}
		return tui.SourceResult{Exists: true, Valid: contErr == nil, Error: errMsg}, nil
	case "continue":
		err := e.Continue(ctx)
		return tui.SourceResult{Exists: true, Valid: err == nil}, err
	case "new", "clone":
		err := e.CreateSource(ctx, req.Action, req.URL)
		return tui.SourceResult{Exists: true, Valid: err == nil}, err
	case "backup-plan":
		p, err := e.PlanBackup()
		var m map[string]any
		_ = json.Unmarshal(encode(p), &m)
		return tui.SourceResult{Exists: true, Backup: p.Backup, Plan: m}, err
	case "replace":
		if req.Plan == nil {
			return tui.SourceResult{}, fmt.Errorf("backup plan required")
		}
		err := e.Replace(*req.Plan, req.Plan.Backup)
		return tui.SourceResult{Backup: req.Plan.Backup}, err
	}
	return tui.SourceResult{}, fmt.Errorf("unsupported source action")
}
func (e *Engine) InitSourceTerminal(ctx context.Context, r map[string]any, s tui.TerminalStreams) (tui.SourceResult, error) {
	var req struct {
		Action string `json:"action"`
		URL    string `json:"url"`
	}
	b, _ := json.Marshal(r)
	if err := native.Decode(b, &req); err != nil {
		return tui.SourceResult{}, fmt.Errorf("invalid terminal source request")
	}
	if req.Action != "clone" {
		return tui.SourceResult{}, fmt.Errorf("only explicit clone uses the Git terminal handoff")
	}
	err := e.CreateSourceTerminal(ctx, req.Action, req.URL, s)
	return tui.SourceResult{Exists: true, Valid: err == nil}, err
}
func (e *Engine) gitAvailable(ctx context.Context) bool {
	path, err := binary(e.options.Git, "git")
	if err != nil {
		return false
	}
	b, err := exec.CommandContext(ctx, path, "--version").Output()
	return err == nil && strings.HasPrefix(string(b), "git version ")
}

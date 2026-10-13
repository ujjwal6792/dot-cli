package setup

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"unicode"

	"github.com/pelletier/go-toml/v2"
	"github.com/ujjwal6792/dots-cli/internal/native"
	"github.com/ujjwal6792/dots-cli/internal/tui"
)

type TrackerPlan struct {
	ID          string `json:"id"`
	Exists      bool   `json:"exists"`
	Text        string `json:"text"`
	Review      string `json:"review"`
	Name        string `json:"name"`
	Email       string `json:"email"`
	Fingerprint string `json:"fingerprint"`
}

func (e *Engine) InitTracker(ctx context.Context, r map[string]any) (tui.SetupResult, error) {
	var req struct {
		Action  string       `json:"action"`
		Name    string       `json:"name,omitempty"`
		Email   string       `json:"email,omitempty"`
		Plan    *TrackerPlan `json:"plan,omitempty"`
		Approve string       `json:"approve,omitempty"`
	}
	b, _ := json.Marshal(r)
	if err := native.Decode(b, &req); err != nil {
		return tui.SetupResult{}, fmt.Errorf("invalid tracker request")
	}
	if req.Action == "apply" {
		if req.Plan == nil {
			return tui.SetupResult{}, fmt.Errorf("tracker plan required")
		}
		return tui.SetupResult{}, e.ApplyTracker(ctx, *req.Plan, req.Approve, true)
	}
	if req.Action != "plan" {
		return tui.SetupResult{}, fmt.Errorf("unsupported tracker action")
	}
	if req.Name == "" {
		req.Name = e.options.Name
	}
	if req.Email == "" {
		req.Email = e.options.Email
	}
	p, err := e.PlanTracker(ctx, req.Name, req.Email)
	var plan map[string]any
	_ = json.Unmarshal(encode(p), &plan)
	return tui.SetupResult{Exists: p.Exists, Plan: plan, Review: p.Review}, err
}

// Complete is a read-only readiness gate, not a cached success bit.
func (e *Engine) Complete(ctx context.Context) error {
	if err := e.noRecovery(); err != nil {
		return err
	}
	if err := e.Continue(ctx); err != nil {
		return err
	}
	if _, err := e.Policy.LoadAdoption(); err != nil {
		return err
	}
	d, err := e.InitDependencies(ctx)
	if err != nil {
		return err
	}
	if !d.Ready {
		return fmt.Errorf("required dependencies missing/incompatible; setup incomplete")
	}
	if ok, err := e.tracker(ctx); err != nil {
		return err
	} else if !ok {
		return fmt.Errorf("tracker missing; setup incomplete")
	}
	_, err = e.ReadConnection()
	return err
}
func hash(b []byte) string { s := sha256.Sum256(b); return hex.EncodeToString(s[:]) }
func (e *Engine) trackerPaths(c native.Contract, m native.Machine) map[string]string {
	return map[string]string{"sourceDir": filepath.Join(e.options.Source, c.HomeSource), "destDir": e.options.Home, "cacheDir": filepath.Join(e.options.State, "chezmoi-cache"), "persistentState": filepath.Join(e.options.State, "chezmoi-state.boltdb")}
}
func (e *Engine) tracker(ctx context.Context) (bool, error) {
	c, err := e.Policy.CheckContract(ctx)
	if err != nil {
		return false, err
	}
	a, err := e.Policy.LoadAdoption()
	if err != nil {
		return false, err
	}
	p := e.options.Config
	if _, err := canonicalPath(p); err != nil {
		return false, err
	}
	for _, ext := range []string{".json", ".jsonc", ".yaml", ".yml"} {
		alternate := strings.TrimSuffix(p, ".toml") + ext
		if _, err := os.Lstat(alternate); err == nil {
			return false, fmt.Errorf("alternate tracker config exists; review explicitly, left untouched")
		} else if !os.IsNotExist(err) {
			return false, err
		}
	}
	b, err := os.ReadFile(p)
	if os.IsNotExist(err) {
		return false, nil
	}
	if err != nil {
		return false, err
	}
	var data map[string]any
	// Do not echo parser diagnostics: tracker data may contain credentials.
	if err := toml.Unmarshal(b, &data); err != nil {
		return false, fmt.Errorf("malformed tracker TOML; left untouched")
	}
	for key, want := range e.trackerPaths(c, a.Machine) {
		if data[key] != want {
			return false, fmt.Errorf("tracker path identity conflict (%s); left untouched", key)
		}
		if _, err := canonicalPath(want); err != nil {
			return false, err
		}
	}
	d, ok := data["data"].(map[string]any)
	if !ok || d["dotMachine"] != a.Machine.ID || d["dotOS"] != a.Machine.OS {
		return false, fmt.Errorf("tracker machine identity conflict; left untouched")
	}
	if _, legacy := d["dotEngine"]; legacy {
		return false, fmt.Errorf("legacy private executable tracker requires explicit migration")
	}
	return true, nil
}
func (e *Engine) gitIdentity(ctx context.Context, key string) (string, error) {
	// Exit 1 means unset, not an invitation to alter user's Git settings.
	cmd := e.gitCommand(ctx, "-C", e.options.Source, "config", "--get", "user."+key)
	b, err := cmd.Output()
	if err != nil {
		var exit interface{ ExitCode() int }
		if errors.As(err, &exit) && exit.ExitCode() == 1 {
			return "", nil
		}
		return "", fmt.Errorf("cannot read Git identity")
	}
	return strings.TrimSpace(string(b)), nil
}
func scalar(s string) bool { return s != "" && strings.IndexFunc(s, unicode.IsControl) < 0 }
func (e *Engine) trackerFingerprint(ctx context.Context) (string, error) {
	c, err := e.Policy.CheckContract(ctx)
	if err != nil {
		return "", err
	}
	var b strings.Builder
	origin, err := e.git(ctx, "-C", e.options.Source, "config", "--get", "remote.origin.url")
	if err != nil {
		return "", err
	}
	b.WriteString(origin + "\x00")
	for _, p := range []string{filepath.Join(e.options.Source, "dot-contract.json"), filepath.Join(e.options.Source, c.Manifest), filepath.Join(e.options.State, "adoption.json"), e.options.Config, filepath.Join(e.options.State, "connection.json")} {
		if _, err := canonicalPath(p); err != nil {
			return "", err
		}
		data, err := os.ReadFile(p)
		if err != nil && !os.IsNotExist(err) {
			return "", err
		}
		fmt.Fprintf(&b, "%s\x00%t\x00%s\x00", p, err == nil, hash(data))
	}
	return hash([]byte(b.String())), nil
}
func (e *Engine) PlanTracker(ctx context.Context, name, email string) (TrackerPlan, error) {
	if err := e.noRecovery(); err != nil {
		return TrackerPlan{}, err
	}
	if err := e.Continue(ctx); err != nil {
		return TrackerPlan{}, err
	}
	a, err := e.Policy.LoadAdoption()
	if err != nil {
		return TrackerPlan{}, err
	}
	if _, err := e.ReadConnection(); err != nil && !errors.Is(err, os.ErrNotExist) {
		return TrackerPlan{}, err
	}
	exists, err := e.tracker(ctx)
	if err != nil {
		return TrackerPlan{}, err
	}
	p := TrackerPlan{Exists: exists}
	p.Fingerprint, err = e.trackerFingerprint(ctx)
	if err != nil {
		return p, err
	}
	if exists {
		p.Review = "Compatible existing tracker detected read-only. Preserve all config/Git/machine data. Confirm to persist this source/HOME/config connection only."
	} else {
		gn, err := e.gitIdentity(ctx, "name")
		if err != nil {
			return p, err
		}
		ge, err := e.gitIdentity(ctx, "email")
		if err != nil {
			return p, err
		}
		if gn != "" {
			name = gn
		}
		if ge != "" {
			email = ge
		}
		if !scalar(name) || !scalar(email) {
			return p, fmt.Errorf("tracker needs name and email: set Git identity or supply tracker-only values; Git settings stay unchanged")
		}
		p.Name, p.Email = name, email
		c, err := e.Policy.CheckContract(ctx)
		if err != nil {
			return p, err
		}
		paths := e.trackerPaths(c, a.Machine)
		quote := func(s string) string { b, _ := json.Marshal(s); return string(b) }
		for _, key := range []string{"sourceDir", "destDir", "cacheDir", "persistentState"} {
			p.Text += key + " = " + quote(paths[key]) + "\n"
		}
		p.Text += "[data]\nname = " + quote(name) + "\nemail = " + quote(email) + "\ndotMachine = " + quote(a.Machine.ID) + "\ndotOS = " + quote(a.Machine.OS) + "\n"
		p.Review = "Create local unmanaged tracker (0600) at " + e.options.Config + "\n" + p.Text + "\nPersist connection in " + e.options.State + ". No templates/hooks, apply, cache/database creation, Git changes, commit or push."
	}
	origin, err := e.git(ctx, "-C", e.options.Source, "config", "--get-all", "remote.origin.url")
	if err != nil {
		return p, err
	}
	p.Review = "Machine: " + a.Machine.ID + " (" + a.Machine.OS + ")\nSource: " + e.options.Source + "\nOrigin: " + origin + "\nHOME: " + e.options.Home + "\nTracker: " + e.options.Config + "\nConnection: " + filepath.Join(e.options.State, "connection.json") + "\n\n" + p.Review
	p.ID = hash(encode(p))
	return p, nil
}
func (e *Engine) ApplyTracker(ctx context.Context, p TrackerPlan, id string, consent bool) error {
	if !consent || id == "" || id != p.ID {
		return fmt.Errorf("exact tracker connection consent required")
	}
	fresh, err := e.PlanTracker(ctx, p.Name, p.Email)
	if err != nil {
		return err
	}
	if fresh.ID != p.ID {
		return fmt.Errorf("stale tracker plan; review again")
	}
	if err := ctx.Err(); err != nil {
		return err
	}
	if !fresh.Exists {
		if err := exclusive(e.options.Config, []byte(fresh.Text)); err != nil {
			return err
		}
	}
	// A publication interrupted here is recoverable by read-only detection followed
	// by a new consent review; never delete the tracker to pretend atomic success.
	if ok, err := e.tracker(ctx); err != nil {
		return err
	} else if !ok {
		return fmt.Errorf("tracker creation not verified")
	}
	a, err := e.Policy.LoadAdoption()
	if err != nil {
		return err
	}
	if err := ctx.Err(); err != nil {
		return err
	}
	return e.connect(a.Machine)
}

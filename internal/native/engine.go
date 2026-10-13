package native

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"sort"
	"strings"
)

type Engine struct{ config Config }

func New(c Config) (*Engine, error) {
	if c.OS == "" {
		c.OS = runtime.GOOS
	}
	if c.OS != "darwin" && c.OS != "linux" {
		return nil, fmt.Errorf("unsupported OS")
	}
	if c.Git == "" {
		c.Git = "git"
	}
	for _, p := range []string{c.Source, c.Home, c.State} {
		if !filepath.IsAbs(p) || filepath.Clean(p) != p {
			return nil, fmt.Errorf("explicit absolute source/home/state paths required")
		}
		if _, err := BoundedPath(p, ".dot-boundary"); err != nil {
			return nil, err
		}
	}
	// A checkout commonly lives inside HOME. Only HOME inside the checkout is
	// forbidden; targetPath separately excludes checkout/state enrollment.
	if within(c.Home, c.Source) || within(c.State, c.Source) || within(c.Source, c.State) {
		return nil, fmt.Errorf("source, target and state must not overlap source")
	}
	return &Engine{config: c}, nil
}
func within(p, root string) bool {
	r, err := filepath.Rel(root, p)
	return err == nil && (r == "." || !strings.HasPrefix(r, ".."+string(filepath.Separator)) && r != "..")
}
func (e *Engine) Config() Config { return e.config }
func (e *Engine) git(ctx context.Context, args ...string) ([]byte, error) {
	cmd := exec.CommandContext(ctx, e.config.Git, append([]string{"-C", e.config.Source}, args...)...)
	cmd.Env = append(os.Environ(), "HOME="+e.config.Home, "XDG_CONFIG_HOME="+filepath.Join(e.config.Home, ".config"), "GIT_TERMINAL_PROMPT=0")
	b, err := cmd.CombinedOutput()
	if err != nil {
		return nil, fmt.Errorf("git %s failed: %w", args[0], err)
	}
	return b, nil
}
func (e *Engine) CheckContract(ctx context.Context) (Contract, error) {
	var c Contract
	if _, err := BoundedPath(e.config.Source, "dot-contract.json"); err != nil {
		return c, err
	}
	root, err := e.git(ctx, "rev-parse", "--show-toplevel")
	if err != nil {
		return c, err
	}
	if filepath.Clean(strings.TrimSpace(string(root))) != e.config.Source {
		return c, fmt.Errorf("source must be the Git checkout root")
	}
	if err := readJSON(e.config.Source, "dot-contract.json", &c); err != nil {
		return c, err
	}
	if c.Protocol != 1 || c.EngineAPI.Min > 1 || c.EngineAPI.Max < 1 || c.EngineAPI.Min < 1 || c.EngineAPI.Max < c.EngineAPI.Min || (c.ManifestVersion != 1 && c.ManifestVersion != 2) || (c.ApprovalVersion != 2 && c.ApprovalVersion != 3) {
		return c, fmt.Errorf("incompatible protocol/manifest/approval capabilities")
	}
	supported := map[string]bool{"intent": true, "scope": true, "gitconfig": true, "skills": true, "layout": true}
	seen := map[string]bool{}
	for _, a := range c.RequiredAdapters {
		if !supported[a] || seen[a] {
			return c, fmt.Errorf("unsupported/duplicate adapter %q", a)
		}
		seen[a] = true
	}
	for _, p := range []string{c.Manifest, c.HomeSource} {
		if _, err := BoundedPath(e.config.Source, p); err != nil {
			return c, err
		}
	}
	homeInfo, err := os.Stat(filepath.Join(e.config.Source, c.HomeSource))
	if err != nil || !homeInfo.IsDir() {
		return c, fmt.Errorf("private home_source must be an existing bounded directory")
	}
	var m Manifest
	if err := readJSON(e.config.Source, c.Manifest, &m); err != nil {
		return c, err
	}
	if m.Version != c.ManifestVersion || m.Entries == nil || m.Profiles == nil {
		return c, fmt.Errorf("manifest version/schema conflicts with contract")
	}
	if err := e.validateManifest(c, m); err != nil {
		return c, err
	}
	return c, nil
}
func (e *Engine) manifest(ctx context.Context) (Contract, Manifest, error) {
	c, err := e.CheckContract(ctx)
	var m Manifest
	if err == nil {
		err = readJSON(e.config.Source, c.Manifest, &m)
	}
	if err == nil {
		if m.Version != c.ManifestVersion || m.Entries == nil || m.Profiles == nil {
			err = fmt.Errorf("manifest changed/incompatible with contract")
		} else {
			err = e.validateManifest(c, m)
		}
	}
	return c, m, err
}
func validateAssignment(a *Assignment) error {
	if a == nil {
		return nil
	}
	if len(a.Include) == 0 || a.Exclude == nil {
		return fmt.Errorf("assignment: include must be nonempty and exclude explicit")
	}
	for _, list := range [][]string{a.Include, a.Exclude} {
		seen := map[string]bool{}
		for _, id := range list {
			if !machineID.MatchString(id) || seen[id] {
				return fmt.Errorf("assignment: invalid/duplicate machine id")
			}
			seen[id] = true
		}
	}
	return nil
}
func has(list []string, s string) bool {
	for _, x := range list {
		if x == s {
			return true
		}
	}
	return false
}
func applicable(x Entry, m Machine) bool {
	if !has(x.OS, m.OS) {
		return false
	}
	a := x.Machines
	if a == nil {
		return true
	}
	return (has(a.Include, "all") || has(a.Include, m.ID)) && !has(a.Exclude, "all") && !has(a.Exclude, m.ID)
}
func (e *Engine) validateEntry(x Entry) error {
	for _, p := range []string{x.Source, x.Destination} {
		if err := relative(p); err != nil {
			return err
		}
	}
	for _, p := range append([]string{x.Destination}, x.Removals...) {
		if err := e.targetPath(p); err != nil {
			return err
		}
	}
	if len(x.OS) == 0 {
		return fmt.Errorf("entry requires explicit OS")
	}
	for _, o := range x.OS {
		if o != "linux" && o != "darwin" {
			return fmt.Errorf("unsupported entry OS")
		}
	}
	if x.Impact != "Safe" && x.Impact != "Unsafe" {
		return fmt.Errorf("entry requires Safe/Unsafe impact")
	}
	if !has([]string{"managed-shared", "managed-local", "managed-templated"}, x.Portability) {
		return fmt.Errorf("unsupported portability")
	}
	if err := validateAssignment(x.Machines); err != nil {
		return err
	}
	if len(x.Hooks) > 0 {
		return fmt.Errorf("private executable hooks are unsupported")
	}
	for _, p := range x.Removals {
		if err := relative(p); err != nil {
			return err
		}
		if strings.EqualFold(p, ".gitconfig") {
			return fmt.Errorf("whole-file Git removal is forbidden")
		}
	}
	for _, i := range x.Integrations {
		if !has([]string{"git-config", "gemini-skills", "claude-skills", "desktop"}, i) {
			return fmt.Errorf("unsupported integration %q", i)
		}
	}
	if strings.EqualFold(x.Destination, ".gitconfig") && (!has(x.Integrations, "git-config") || x.Destination != ".gitconfig") {
		return fmt.Errorf("whole-file .gitconfig replacement is forbidden")
	}
	if has(x.Integrations, "git-config") {
		if x.Destination != ".gitconfig" || len(x.GitKeys) == 0 || x.Portability != "managed-templated" {
			return fmt.Errorf("bounded Git metadata required")
		}
		for _, k := range x.GitKeys {
			if k != "user.name" && k != "user.email" {
				return fmt.Errorf("unapproved Git key")
			}
		}
		for k := range x.GitValues {
			if !has(x.GitKeys, k) {
				return fmt.Errorf("Git value outside selected keys")
			}
		}
	} else if len(x.GitKeys) > 0 || len(x.GitValues) > 0 || x.GitSource != "" {
		return fmt.Errorf("Git metadata requires bounded integration")
	}
	if x.GitSource != "" {
		if err := relative(x.GitSource); err != nil {
			return err
		}
	}
	return nil
}
func (e *Engine) targetPath(p string) error {
	if err := relative(p); err != nil {
		return err
	}
	target := filepath.Join(e.config.Home, p)
	if within(target, e.config.State) || within(target, e.config.Source) || target == e.config.TrackerConfig || sensitive(p) {
		return fmt.Errorf("target overlaps private state/source or sensitive runtime: %s", p)
	}
	return nil
}
func (e *Engine) validateManifest(c Contract, m Manifest) error {
	destinations := map[string]string{}
	for id, x := range m.Entries {
		if id == "" {
			return fmt.Errorf("empty manifest id")
		}
		if err := e.validateEntry(x); err != nil {
			return fmt.Errorf("entry %s: %w", id, err)
		}
		if !within(filepath.Join(e.config.Source, x.Source), filepath.Join(e.config.Source, c.HomeSource)) {
			return fmt.Errorf("source outside private home boundary")
		}
		if _, err := BoundedPath(e.config.Source, x.Source); err != nil {
			return err
		}
		if x.GitSource != "" {
			if !within(filepath.Join(e.config.Source, x.GitSource), filepath.Join(e.config.Source, c.HomeSource)) {
				return fmt.Errorf("Git source outside home boundary")
			}
			if _, err := BoundedPath(e.config.Source, x.GitSource); err != nil {
				return err
			}
		}
		for _, o := range x.OS {
			key := o + ":" + x.Destination
			if old, ok := destinations[key]; ok && old != id {
				return fmt.Errorf("conflicting destination: %s", x.Destination)
			}
			destinations[key] = id
		}
	}
	for _, ids := range m.Profiles {
		seen := map[string]bool{}
		for _, id := range ids {
			if _, ok := m.Entries[id]; !ok {
				return fmt.Errorf("profile refers to unknown entry %q", id)
			}
			if seen[id] {
				return fmt.Errorf("duplicate profile entry")
			}
			seen[id] = true
		}
	}
	for id, mi := range m.Machines {
		if err := ValidMachine(Machine{ID: id, OS: mi.OS}); err != nil {
			return err
		}
	}
	for id, l := range m.Tracking {
		if id == "" {
			return fmt.Errorf("empty logical id")
		}
		if err := validateAssignment(l.Machines); err != nil {
			return err
		}
		for _, p := range l.Destinations {
			if err := relative(p); err != nil {
				return err
			}
		}
		for _, p := range l.Sources {
			if _, err := BoundedPath(e.config.Source, p); err != nil {
				return err
			}
		}
	}
	return e.validateLogical(c, m)
}
func digest(v any) string {
	b, _ := json.Marshal(v)
	h := sha256.Sum256(b)
	return hex.EncodeToString(h[:])
}
func sortedKeys[V any](m map[string]V) []string {
	out := make([]string, 0, len(m))
	for k := range m {
		out = append(out, k)
	}
	sort.Strings(out)
	return out
}

package native

import (
	"context"
	"encoding/json"
	"fmt"
	"io/fs"
	"path/filepath"
	"sort"
	"strings"
)

func metadata(x Entry) map[string]any {
	b, _ := json.Marshal(x)
	var m map[string]any
	_ = json.Unmarshal(b, &m)
	delete(m, "source")
	delete(m, "logical_id")
	return m
}
func sensitive(path string) bool {
	for _, part := range strings.Split(path, "/") {
		if has([]string{".ssh", ".aws", ".gnupg", ".cache", ".git", ".netrc", "auth.json", "credentials", "credentials.json", "id_rsa", "id_ed25519"}, part) {
			return true
		}
	}
	return strings.HasPrefix(path, ".local/state/") || strings.HasPrefix(path, ".config/chezmoi/")
}
func (e *Engine) InventoryMode(ctx context.Context, mode string) (Inventory, error) {
	result := Inventory{OS: e.config.OS, Entries: []InventoryEntry{}, Profiles: []Profile{}, Machines: []string{}}
	if !has([]string{"track", "adopt", "untrack"}, mode) {
		return result, fmt.Errorf("invalid inventory mode")
	}
	_, m, err := e.manifest(ctx)
	if err != nil {
		return result, err
	}
	a, err := e.LoadAdoption()
	if err != nil {
		return result, err
	}
	result.Machine = a.Machine
	m, err = resolve(m, a.Machine)
	if err != nil {
		return result, err
	}
	machines := map[string]bool{a.Machine.ID: true}
	for id := range m.Machines {
		machines[id] = true
	}
	byPath := map[string]InventoryEntry{}
	for _, id := range sortedKeys(m.Entries) {
		x := m.Entries[id]
		x, err = customize(x, a.Customs[id])
		if err != nil {
			return result, err
		}
		if x.Machines != nil {
			for _, id := range append(append([]string{}, x.Machines.Include...), x.Machines.Exclude...) {
				if id != "all" {
					machines[id] = true
				}
			}
		}
		_, approved := a.Entries[id]
		osStatus := "unsupported"
		if applicable(x, a.Machine) {
			osStatus = "supported"
		}
		target, err := readFile(e.config.Home, x.Destination)
		if err != nil {
			return result, err
		}
		status := "missing"
		if target.Exists {
			status = "present"
		}
		if applicable(x, a.Machine) {
			desired, err := e.entryContent(ctx, x)
			if err != nil {
				return result, err
			}
			status = "changed"
			if !target.Exists {
				status = "missing"
			} else if digest(target) == digest(desired) {
				status = "unchanged"
			}
		}
		item := InventoryEntry{Path: x.Destination, ID: id, Kind: "file", Status: "managed", Destination: x.Destination, Metadata: metadata(x), LogicalID: x.LogicalID, OSStatus: osStatus, TargetStatus: status, InScope: approved, Approved: approved}
		if l, ok := m.Tracking[x.LogicalID]; ok {
			b, _ := json.Marshal(l)
			_ = json.Unmarshal(b, &item.Tracking)
		}
		if previous, exists := byPath[x.Destination]; exists {
			if item.OSStatus == "supported" {
				delete(byPath, x.Destination)
				previous.Path += " [" + previous.ID + "]"
				byPath[previous.Path] = previous
			} else {
				item.Path += " [" + id + "]"
			}
		}
		byPath[item.Path] = item
	}
	if mode == "track" {
		err = filepath.WalkDir(e.config.Home, func(path string, d fs.DirEntry, err error) error {
			if err != nil {
				return err
			}
			if path == e.config.Home {
				return nil
			}
			rel, _ := filepath.Rel(e.config.Home, path)
			if within(path, e.config.State) || within(path, e.config.Source) || path == e.config.TrackerConfig || sensitive(rel) {
				if d.IsDir() {
					return filepath.SkipDir
				}
				return nil
			}
			if _, exists := byPath[rel]; exists {
				return nil
			}
			status := "eligible"
			kind := "file"
			if d.IsDir() {
				kind = "directory"
			} else if d.Type()&fs.ModeSymlink != 0 || !d.Type().IsRegular() {
				status = "restricted"
			}
			if rel == ".gitconfig" {
				status = "restricted"
			}
			byPath[rel] = InventoryEntry{Path: rel, Kind: kind, Status: status, Metadata: map[string]any{"destination": rel, "machines": map[string]any{"include": []string{"all"}, "exclude": []string{}}}}
			return nil
		})
		if err != nil {
			return result, err
		}
	}
	// Navigation parents never carry an ID or authorization.
	for path := range byPath {
		for parent := filepath.Dir(path); parent != "."; parent = filepath.Dir(parent) {
			if _, ok := byPath[parent]; !ok {
				byPath[parent] = InventoryEntry{Path: parent, Kind: "directory", Status: "navigation"}
			}
		}
	}
	for _, path := range sortedKeys(byPath) {
		result.Entries = append(result.Entries, byPath[path])
	}
	result.Machines = sortedKeys(machines)
	for _, name := range sortedKeys(m.Profiles) {
		ids := []string{}
		for _, id := range m.Profiles[name] {
			if applicable(m.Entries[id], a.Machine) && !has(a.Deselected, id) {
				ids = append(ids, id)
			}
		}
		result.Profiles = append(result.Profiles, Profile{Name: name, Entries: ids})
	}
	return result, nil
}
func (e *Engine) Status(ctx context.Context) (Inventory, error) { return e.InventoryMode(ctx, "adopt") }
func (e *Engine) ScopeShow(ctx context.Context) (Adoption, error) {
	_, m, err := e.manifest(ctx)
	if err != nil {
		return Adoption{}, err
	}
	a, err := e.LoadAdoption()
	if err != nil {
		return a, err
	}
	_, err = resolve(m, a.Machine)
	return a, err
}
func (e *Engine) Diff(ctx context.Context) ([]Diff, error) {
	_, m, err := e.manifest(ctx)
	if err != nil {
		return nil, err
	}
	a, err := e.LoadAdoption()
	if err != nil {
		return nil, err
	}
	p := Plan{Direction: "repository → target", Actions: []Action{}}
	m, err = resolve(m, a.Machine)
	if err != nil {
		return nil, err
	}
	for _, id := range sortedKeys(m.Entries) {
		x := m.Entries[id]
		if !applicable(x, a.Machine) {
			continue
		}
		x, err = customize(x, a.Customs[id])
		if err != nil {
			return nil, err
		}
		if has(x.Integrations, "git-config") {
			err = e.addGitAction(ctx, &p, x)
		} else {
			s, readErr := e.entryContent(ctx, x)
			if readErr != nil {
				return nil, readErr
			}
			err = e.addAction(&p, "home", x.Destination, s)
		}
		if err != nil {
			return nil, err
		}
	}
	return PlanDiffs(p), nil
}

type SnapshotResult struct {
	Paths   []string `json:"paths"`
	Skipped int      `json:"skipped"`
}

func (e *Engine) Snapshot(ctx context.Context, folder string) (SnapshotResult, error) {
	result := SnapshotResult{Paths: []string{}}
	if err := relative(folder); err != nil {
		return result, err
	}
	inv, err := e.InventoryMode(ctx, "track")
	if err != nil {
		return result, err
	}
	for _, x := range inv.Entries {
		if x.Kind == "file" && strings.HasPrefix(x.Path, folder+"/") {
			if x.Status == "eligible" || x.Status == "managed" && x.Path != ".gitconfig" {
				result.Paths = append(result.Paths, x.Path)
			} else {
				result.Skipped++
			}
		}
	}
	sort.Strings(result.Paths)
	return result, nil
}

type HighlightResult struct {
	Status    string `json:"status"`
	Direction string `json:"direction"`
	Summary   string `json:"summary"`
	Text      string `json:"text"`
	Review    string `json:"review"`
	Diffs     []Diff `json:"diffs"`
}

func (e *Engine) Highlight(ctx context.Context, mode, path string, r *Request) (HighlightResult, error) {
	inv, err := e.InventoryMode(ctx, mode)
	if err != nil {
		return HighlightResult{}, err
	}
	var entry InventoryEntry
	found := false
	for _, x := range inv.Entries {
		if x.Path == path {
			entry = x
			found = true
			break
		}
	}
	if !found {
		return HighlightResult{}, fmt.Errorf("unknown inventory path")
	}
	out := HighlightResult{Status: entry.Status, Summary: "Browsing is not authorization", Direction: "home → repository", Diffs: []Diff{}}
	if mode == "adopt" {
		out.Direction = "repository → target"
		diffs, err := e.Diff(ctx)
		if err != nil {
			return out, err
		}
		for _, d := range diffs {
			if d.Path == path {
				out.Diffs = append(out.Diffs, d)
			}
		}
	} else if r != nil {
		p, err := e.Plan(ctx, *r)
		if err != nil {
			return out, err
		}
		out.Diffs = PlanDiffs(p)
	}
	return out, nil
}

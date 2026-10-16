package native

import (
	"context"
	"encoding/json"
	"fmt"
	"io/fs"
	"os"
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
	return e.inventoryMode(ctx, mode, false)
}

// BrowserInventory keeps the interactive Track browser shallow. Scripted
// inventory and exact folder snapshots still use the complete inventory.
func (e *Engine) BrowserInventory(ctx context.Context, mode string) (Inventory, error) {
	return e.inventoryMode(ctx, mode, true)
}

func (e *Engine) inventoryMode(ctx context.Context, mode string, shallow bool) (Inventory, error) {
	result := Inventory{OS: e.config.OS, Entries: []InventoryEntry{}, Profiles: []Profile{}, Machines: []string{}}
	if err := ctx.Err(); err != nil {
		return result, err
	}
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
	place := func(item InventoryEntry, id string) {
		if l, ok := m.Tracking[item.LogicalID]; ok {
			b, _ := json.Marshal(l)
			_ = json.Unmarshal(b, &item.Tracking)
		}
		if previous, exists := byPath[item.Destination]; exists {
			if item.OSStatus == "supported" {
				delete(byPath, item.Destination)
				previous.Path += " [" + previous.ID + "]"
				byPath[previous.Path] = previous
			} else {
				item.Path += " [" + id + "]"
			}
		}
		byPath[item.Path] = item
	}
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
		md := metadata(x)
		if applicable(x, a.Machine) {
			desired, err := e.entryContent(ctx, x)
			if err != nil {
				// One unreadable entry must not blank the whole browser:
				// flag it and keep listing. Planning still fails closed
				// when the flagged entry is selected.
				md["error"] = err.Error()
				place(InventoryEntry{Path: x.Destination, ID: id, Kind: "file", Status: "managed", Destination: x.Destination, Metadata: md, LogicalID: x.LogicalID, OSStatus: osStatus, TargetStatus: "error", InScope: approved, Approved: approved}, id)
				continue
			}
			status = "changed"
			if !target.Exists {
				status = "missing"
			} else if digest(target) == digest(desired) {
				status = "unchanged"
			}
		}
		place(InventoryEntry{Path: x.Destination, ID: id, Kind: "file", Status: "managed", Destination: x.Destination, Metadata: md, LogicalID: x.LogicalID, OSStatus: osStatus, TargetStatus: status, InScope: approved, Approved: approved}, id)
	}
	if mode == "track" {
		err = filepath.WalkDir(e.config.Home, func(path string, d fs.DirEntry, err error) error {
			if err != nil {
				return err
			}
			if err := ctx.Err(); err != nil {
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
			// Never-dot caches/build outputs: walking them is pure syscall
			// cost with no Track value. Top-level names only; a user file
			// that happens to share one of these exact names at a deeper
			// level is still listed because only its parent dir is skipped
			// when the parent itself is the cache root.
			if excludedBrowserDirectory(filepath.Base(path), d.IsDir()) {
				return filepath.SkipDir
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
			if shallow && d.IsDir() {
				return filepath.SkipDir
			}
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

// BrowseDirectory reads only the immediate children of an expanded HOME folder.
// No content is read and symlink/private-state boundaries match inventory.
func (e *Engine) BrowseDirectory(ctx context.Context, folder string) ([]InventoryEntry, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	if err := e.targetPath(folder); err != nil {
		return nil, err
	}
	path, err := BoundedPath(e.config.Home, folder)
	if err != nil {
		return nil, err
	}
	children, err := os.ReadDir(path)
	if err != nil {
		return nil, err
	}
	entries := make([]InventoryEntry, 0, len(children))
	for _, child := range children {
		if err := ctx.Err(); err != nil {
			return nil, err
		}
		rel := filepath.Join(folder, child.Name())
		if e.targetPath(rel) != nil || excludedBrowserDirectory(child.Name(), child.IsDir()) {
			continue
		}
		kind, status := "file", "eligible"
		if child.IsDir() {
			kind = "directory"
		} else if !child.Type().IsRegular() {
			status = "restricted"
		}
		entries = append(entries, InventoryEntry{Path: rel, Kind: kind, Status: status, Metadata: map[string]any{"destination": rel, "machines": map[string]any{"include": []string{"all"}, "exclude": []string{}}}})
	}
	return entries, nil
}

func excludedBrowserDirectory(name string, directory bool) bool {
	return directory && has([]string{"node_modules", "__pycache__", ".venv", "target", ".next", ".git"}, name)
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
	// Fast path: single-entry diff without a full inventory rescan. The old
	// implementation called InventoryMode (full manifest scan + HOME walk in
	// track mode) plus Diff/Plan (second full scan) per cursor move, so
	// browsing a 300-entry checkout spawned repeated `git rev-parse`
	// processes and thousands of Lstat/read syscalls. This path reads only
	// the manifest, adoption, and the two files for `path`.
	out := HighlightResult{Status: "unknown", Summary: "Browsing is not authorization", Direction: "home → repository", Diffs: []Diff{}}
	if err := ctx.Err(); err != nil {
		return out, err
	}
	if mode == "adopt" {
		out.Direction = "repository → target"
	}
	if err := relative(path); err != nil {
		return out, err
	}
	_, m, err := e.manifest(ctx)
	if err != nil {
		return out, err
	}
	a, err := e.LoadAdoption()
	if err != nil {
		return out, err
	}
	m, err = resolve(m, a.Machine)
	if err != nil {
		return out, err
	}
	// Adopt/status/diff view: managed entry by destination.
	for _, id := range sortedKeys(m.Entries) {
		x := m.Entries[id]
		if x.Destination != path {
			continue
		}
		if _, ok := a.Entries[id]; ok {
			out.Status = "managed"
		} else if applicable(x, a.Machine) {
			out.Status = "eligible"
		} else {
			out.Status = "restricted"
		}
		if !applicable(x, a.Machine) {
			return out, nil
		}
		x, err = customize(x, a.Customs[id])
		if err != nil {
			return out, err
		}
		p := Plan{Direction: out.Direction, Actions: []Action{}}
		if has(x.Integrations, "git-config") {
			err = e.addGitAction(ctx, &p, x)
		} else {
			s, readErr := e.entryContent(ctx, x)
			if readErr != nil {
				return out, readErr
			}
			err = e.addAction(&p, "home", x.Destination, s)
		}
		if err != nil {
			return out, err
		}
		out.Diffs = PlanDiffs(p)
		return out, nil
	}
	// Track view: untracked HOME path. No content diff; report eligibility
	// without walking the whole HOME tree.
	target, err := readFile(e.config.Home, path)
	if err != nil {
		return out, err
	}
	if !target.Exists {
		return out, fmt.Errorf("unknown inventory path")
	}
	out.Status = "eligible"
	if mode == "adopt" {
		return out, nil
	}
	if r != nil {
		p, err := e.Plan(ctx, *r)
		if err != nil {
			return out, err
		}
		out.Diffs = PlanDiffs(p)
	}
	return out, nil
}

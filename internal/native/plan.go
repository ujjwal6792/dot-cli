package native

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"os"
	"sort"
	"strings"
)

func validateCustom(c map[string]any) error {
	for k, v := range c {
		if !has([]string{"os", "tools", "warnings", "notices", "requires", "impact", "removals", "integrations", "git_keys"}, k) {
			return fmt.Errorf("local custom cannot alter capability/mapping %q", k)
		}
		if k == "impact" {
			if v != "Unsafe" && v != "Safe" {
				return fmt.Errorf("invalid custom impact")
			}
		} else {
			b, _ := json.Marshal(v)
			var list []string
			if err := Decode(b, &list); err != nil || list == nil {
				return fmt.Errorf("custom %s must be a string list", k)
			}
			seen := map[string]bool{}
			for _, item := range list {
				if seen[item] {
					return fmt.Errorf("duplicate custom %s", k)
				}
				seen[item] = true
			}
		}
	}
	return nil
}
func customize(x Entry, c map[string]any) (Entry, error) {
	if err := validateCustom(c); err != nil {
		return x, err
	}
	b, _ := json.Marshal(x)
	var m map[string]any
	_ = json.Unmarshal(b, &m)
	for k, v := range c {
		if k == "impact" && x.Impact == "Unsafe" && v == "Safe" {
			return x, fmt.Errorf("custom cannot weaken Unsafe gate")
		}
		m[k] = v
	}
	b, _ = json.Marshal(m)
	err := Decode(b, &x)
	return x, err
}
func (e *Engine) Plan(ctx context.Context, r Request) (Plan, error) {
	// Plans cross the serialized CLI/view boundary. Normalize omitted empty
	// metadata maps once and own the request so recomputation has identical
	// semantics and caller-owned maps cannot be mutated during planning.
	b, err := json.Marshal(r)
	if err != nil {
		return Plan{}, err
	}
	var canonical Request
	if err := Decode(b, &canonical); err != nil {
		return Plan{}, err
	}
	r = canonical
	p := Plan{Version: 1, Request: r, Selected: map[string]Entry{}, Actions: []Action{}}
	if r.Version != 1 || !has([]string{"adopt", "update", "track", "untrack", "metadata"}, r.Mode) || r.Profile == "" {
		return p, fmt.Errorf("invalid intent protocol/mode/profile")
	}
	if r.Mode != "track" && len(r.Selections) > 0 {
		return p, fmt.Errorf("selections are only valid in exact Track")
	}
	seenProfiles := map[string]bool{}
	for _, name := range r.Profiles {
		if name == "" || seenProfiles[name] {
			return p, fmt.Errorf("invalid duplicate/empty profile")
		}
		seenProfiles[name] = true
	}
	if r.Mode == "update" && len(r.Profiles) > 0 {
		return p, fmt.Errorf("update preserves existing exact profile composition")
	}
	c, m, err := e.manifest(ctx)
	if err != nil {
		return p, err
	}
	a, err := e.LoadAdoption()
	if err != nil {
		return p, err
	}
	p.Machine = a.Machine
	state, err := readFile(e.config.State, "adoption.json")
	if err != nil {
		return p, err
	}
	contract, err := readFile(e.config.Source, "dot-contract.json")
	if err != nil {
		return p, err
	}
	manifest, err := readFile(e.config.Source, c.Manifest)
	if err != nil {
		return p, err
	}
	head, _ := e.git(ctx, "rev-parse", "--verify", "--quiet", "HEAD")
	p.Fingerprint = digest([]any{e.config, state, contract, manifest, head})
	if r.Mode == "metadata" {
		return e.planMetadata(p, c, m, a)
	}
	if r.Mode == "track" || r.Mode == "untrack" {
		return e.planSource(ctx, p, c, m, a)
	}
	m, err = resolve(m, a.Machine)
	if err != nil {
		return p, err
	}
	p.Direction = "repository → target"
	profiles := append([]string{}, r.Profiles...)
	if len(profiles) == 0 {
		if len(a.Profiles) > 0 && r.Profile == a.Profile {
			profiles = append(profiles, a.Profiles...)
		} else if r.Profile != "" {
			if _, ok := m.Profiles[r.Profile]; ok {
				profiles = []string{r.Profile}
			} else if r.Entries == nil {
				return p, fmt.Errorf("unknown named profile %q", r.Profile)
			}
		}
	}
	sort.Strings(profiles)
	pool := map[string]bool{}
	for _, name := range profiles {
		ids, ok := m.Profiles[name]
		if !ok {
			return p, fmt.Errorf("unknown named profile %q", name)
		}
		for _, id := range ids {
			pool[id] = true
		}
	}
	ids := r.Entries
	if r.Mode == "update" {
		if r.Entries != nil || len(r.Metadata) > 0 || len(r.Selections) > 0 {
			return p, fmt.Errorf("update uses existing exact approval only")
		}
		ids = sortedKeys(a.Entries)
	} else if ids == nil {
		for _, id := range sortedKeys(pool) {
			if applicable(m.Entries[id], a.Machine) && !has(a.Deselected, id) {
				ids = append(ids, id)
			}
		}
	}
	seen := map[string]bool{}
	for _, id := range ids {
		if seen[id] {
			return p, fmt.Errorf("duplicate selected id")
		}
		seen[id] = true
		x, ok := m.Entries[id]
		if !ok || !applicable(x, a.Machine) {
			return p, fmt.Errorf("explicit entry %q is unknown/inapplicable", id)
		}
		base := x
		x, err = customize(x, a.Customs[id])
		if err != nil {
			return p, err
		}
		if local, ok := r.Metadata[id]; ok {
			x, err = customize(x, local)
			if err != nil {
				return p, err
			}
		}
		if r.Mode == "update" {
			old := a.Entries[id]
			if x.Destination != old.Destination || x.Source != old.Source || digest(x.GitKeys) != digest(old.GitKeys) || digest(x.Removals) != digest(old.Removals) || digest(x.Integrations) != digest(old.Integrations) {
				return p, fmt.Errorf("approved capabilities/mapping changed; review Adopt")
			}
		}
		if !applicable(x, a.Machine) {
			return p, fmt.Errorf("customized entry is not applicable")
		}
		if err = e.validateEntry(x); err != nil {
			return p, err
		}
		if base.Impact == "Unsafe" {
			p.Gates.Unsafe = true
		}
		if len(base.Warnings) > 0 || has(base.Integrations, "desktop") {
			p.Gates.Desktop = true
		}
		p.Selected[id] = x
	}
	for id := range r.Metadata {
		if !seen[id] {
			return p, fmt.Errorf("custom metadata outside exact selection")
		}
	}
	for _, id := range sortedKeys(p.Selected) {
		x := p.Selected[id]
		var s FileState
		if has(x.Integrations, "git-config") {
			err = e.addGitAction(ctx, &p, x)
		} else {
			s, err = e.entryContent(ctx, x)
			if err == nil {
				err = e.addAction(&p, "home", x.Destination, s)
			}
		}
		if err != nil {
			return p, err
		}
		if x.Impact == "Unsafe" {
			p.Gates.Unsafe = true
		}
		if has(x.Integrations, "desktop") || len(x.Warnings) > 0 {
			p.Gates.Desktop = true
		}
		for _, path := range x.Removals {
			if err = e.addAction(&p, "home", path, FileState{}); err != nil {
				return p, err
			}
		}
		for _, integration := range x.Integrations {
			var mirror string
			switch integration {
			case "gemini-skills":
				mirror = ".gemini/skills/"
			case "claude-skills":
				mirror = ".claude/skills/"
			}
			if mirror != "" {
				const canonical = ".agents/skills/"
				if len(x.Destination) <= len(canonical) || x.Destination[:len(canonical)] != canonical {
					return p, fmt.Errorf("skills integration requires canonical exact skill file")
				}
				if err = e.addAction(&p, "home", mirror+x.Destination[len(canonical):], s); err != nil {
					return p, err
				}
			}
		}
		p.Notices = append(p.Notices, x.Notices...)
	}
	next := a
	next.Profile = r.Profile
	if r.Mode == "update" {
		next.Profile = a.Profile
	}
	next.Entries = p.Selected
	if r.Mode == "adopt" {
		next.Profiles = profiles
		// Exclusions are machine-local intent, not just the current profile's
		// complement. Explicit reselection clears an exclusion; switching away
		// from a profile must not silently forget it.
		next.Deselected = []string{}
		for _, id := range a.Deselected {
			if !seen[id] {
				next.Deselected = append(next.Deselected, id)
			}
		}
		for _, id := range sortedKeys(pool) {
			if applicable(m.Entries[id], a.Machine) && !seen[id] && !has(next.Deselected, id) {
				next.Deselected = append(next.Deselected, id)
			}
		}
		sort.Strings(next.Deselected)
		if next.Customs == nil {
			next.Customs = map[string]map[string]any{}
		}
		for id, v := range r.Metadata {
			next.Customs[id] = v
		}
	}
	if err = e.addAction(&p, "state", "adoption.json", jsonState(next)); err != nil {
		return p, err
	}
	p.ID = planID(p)
	return p, nil
}
func planID(p Plan) string { p.ID = ""; return digest(p) }
func (e *Engine) root(name string) (string, error) {
	switch name {
	case "home":
		return e.config.Home, nil
	case "source":
		return e.config.Source, nil
	case "state":
		return e.config.State, nil
	}
	return "", fmt.Errorf("unknown action root")
}
func (e *Engine) addAction(p *Plan, root, path string, after FileState) error {
	if root == "home" {
		if err := e.targetPath(path); err != nil {
			return err
		}
	}
	r, err := e.root(root)
	if err != nil {
		return err
	}
	before, err := readFile(r, path)
	if err != nil {
		return err
	}
	for _, a := range p.Actions {
		if a.Root == root && a.Path == path {
			if digest(a.After) == digest(after) {
				return nil
			}
			return fmt.Errorf("conflicting exact actions: %s", path)
		}
	}
	a := Action{Root: root, Path: path, Before: before, After: after}
	if (root == "home" || root == "source") && before.Exists && digest(before) != digest(after) {
		a.BackupPath = path + ".dot-backup"
		a.BackupBefore, err = readFile(r, a.BackupPath)
		if err != nil {
			return err
		}
		p.Gates.Backup = p.Gates.Backup || a.BackupBefore.Exists
	}
	for _, old := range p.Actions {
		if old.Root == root && (path == old.BackupPath || a.BackupPath != "" && a.BackupPath == old.Path) {
			return fmt.Errorf("action conflicts with exact backup slot")
		}
	}
	p.Actions = append(p.Actions, a)
	return nil
}
func (e *Engine) entryContent(ctx context.Context, x Entry) (FileState, error) {
	if has(x.Integrations, "git-config") {
		return e.gitContent(ctx, x)
	}
	s, err := readFile(e.config.Source, x.Source)
	if err != nil {
		return s, err
	}
	if !s.Exists {
		return s, fmt.Errorf("missing source %s", x.Source)
	}
	if x.Portability == "managed-templated" {
		return e.renderTemplate(x, s)
	}
	return s, nil
}
func (e *Engine) Preview(ctx context.Context, p Plan) ([]Diff, error) {
	fresh, err := e.Plan(ctx, p.Request)
	if err != nil {
		return nil, err
	}
	if p.ID != planID(p) || p.ID != fresh.ID {
		return nil, fmt.Errorf("stale/changed plan")
	}
	return PlanDiffs(p), nil
}
func PlanDiffs(p Plan) []Diff {
	out := []Diff{}
	for _, a := range p.Actions {
		if a.Root == "state" {
			continue
		}
		status := "unchanged"
		if digest(a.Before) != digest(a.After) {
			status = "write"
			if !a.After.Exists {
				status = "remove"
			}
		}
		text := ""
		if a.Path != ".gitconfig" && len(a.Before.Data)+len(a.After.Data) < 128*1024 {
			text = computeLineDiff(a.Before.Data, a.After.Data, 3)
		} else if a.Path == ".gitconfig" && len(a.GitValues) > 0 {
			var b strings.Builder
			b.WriteString("--- before\n+++ after\n")
			for _, k := range sortedKeys(a.GitValues) {
				fmt.Fprintf(&b, "@@ %s @@\n+%s\n", k, a.GitValues[k])
			}
			text = strings.TrimRight(b.String(), "\n")
		}
		out = append(out, Diff{Path: a.Path, Status: status, Summary: p.Direction, Text: text})
	}
	return out
}

type diffOp int

const (
	diffEqual diffOp = iota
	diffDelete
	diffInsert
)

type diffEdit struct {
	op   diffOp
	text string
}

func computeLineDiff(before, after []byte, contextLines int) string {
	if bytes.Equal(before, after) {
		return ""
	}
	if len(before) == 0 && len(after) == 0 {
		return ""
	}

	splitLines := func(data []byte) []string {
		if len(data) == 0 {
			return nil
		}
		s := string(data)
		lines := strings.Split(s, "\n")
		if len(lines) > 0 && lines[len(lines)-1] == "" {
			lines = lines[:len(lines)-1]
		}
		return lines
	}

	a := splitLines(before)
	b := splitLines(after)

	if len(a) == 0 {
		var buf strings.Builder
		fmt.Fprintf(&buf, "--- before\n+++ after\n@@ -0,0 +1,%d @@\n", len(b))
		for _, line := range b {
			buf.WriteString("+" + line + "\n")
		}
		return strings.TrimRight(buf.String(), "\n")
	}

	if len(b) == 0 {
		var buf strings.Builder
		fmt.Fprintf(&buf, "--- before\n+++ after\n@@ -1,%d +0,0 @@\n", len(a))
		for _, line := range a {
			buf.WriteString("-" + line + "\n")
		}
		return strings.TrimRight(buf.String(), "\n")
	}

	// Dynamic programming LCS for edit script
	n := len(a)
	m := len(b)
	dp := make([][]int, n+1)
	for idx := range dp {
		dp[idx] = make([]int, m+1)
	}
	for i := n - 1; i >= 0; i-- {
		for j := m - 1; j >= 0; j-- {
			if a[i] == b[j] {
				dp[i][j] = dp[i+1][j+1] + 1
			} else if dp[i+1][j] >= dp[i][j+1] {
				dp[i][j] = dp[i+1][j]
			} else {
				dp[i][j] = dp[i][j+1]
			}
		}
	}

	var edits []diffEdit
	i, j := 0, 0
	for i < n || j < m {
		if i < n && j < m && a[i] == b[j] {
			edits = append(edits, diffEdit{op: diffEqual, text: a[i]})
			i++
			j++
		} else if i < n && (j == m || dp[i+1][j] >= dp[i][j+1]) {
			edits = append(edits, diffEdit{op: diffDelete, text: a[i]})
			i++
		} else {
			edits = append(edits, diffEdit{op: diffInsert, text: b[j]})
			j++
		}
	}

	type hunkItem struct {
		edit      diffEdit
		beforeNum int
		afterNum  int
	}

	var items []hunkItem
	bNum, aNum := 1, 1
	for _, ed := range edits {
		item := hunkItem{edit: ed}
		if ed.op == diffEqual {
			item.beforeNum = bNum
			item.afterNum = aNum
			bNum++
			aNum++
		} else if ed.op == diffDelete {
			item.beforeNum = bNum
			bNum++
		} else {
			item.afterNum = aNum
			aNum++
		}
		items = append(items, item)
	}

	var changeIndices []int
	for idx, it := range items {
		if it.edit.op != diffEqual {
			changeIndices = append(changeIndices, idx)
		}
	}
	if len(changeIndices) == 0 {
		return ""
	}

	type rangePair struct {
		start int
		end   int
	}
	var hunkRanges []rangePair
	curStart := max(0, changeIndices[0]-contextLines)
	curEnd := min(len(items), changeIndices[0]+contextLines+1)

	for _, chIdx := range changeIndices[1:] {
		hunkStart := max(0, chIdx-contextLines)
		hunkEnd := min(len(items), chIdx+contextLines+1)
		if hunkStart <= curEnd {
			if hunkEnd > curEnd {
				curEnd = hunkEnd
			}
		} else {
			hunkRanges = append(hunkRanges, rangePair{curStart, curEnd})
			curStart = hunkStart
			curEnd = hunkEnd
		}
	}
	hunkRanges = append(hunkRanges, rangePair{curStart, curEnd})

	var buf strings.Builder
	buf.WriteString("--- before\n+++ after\n")
	for _, hr := range hunkRanges {
		slice := items[hr.start:hr.end]
		bStart, bCount := 0, 0
		aStart, aCount := 0, 0
		for _, it := range slice {
			if it.edit.op == diffEqual || it.edit.op == diffDelete {
				if bStart == 0 {
					bStart = it.beforeNum
				}
				bCount++
			}
			if it.edit.op == diffEqual || it.edit.op == diffInsert {
				if aStart == 0 {
					aStart = it.afterNum
				}
				aCount++
			}
		}
		fmt.Fprintf(&buf, "@@ -%d,%d +%d,%d @@\n", bStart, bCount, aStart, aCount)
		for _, it := range slice {
			prefix := " "
			if it.edit.op == diffDelete {
				prefix = "-"
			} else if it.edit.op == diffInsert {
				prefix = "+"
			}
			buf.WriteString(prefix + it.edit.text + "\n")
		}
	}
	return strings.TrimRight(buf.String(), "\n")
}
func (e *Engine) Apply(ctx context.Context, p Plan, consent Consent) error {
	if consent.Approve == "" || consent.Approve != p.ID || p.ID != planID(p) {
		return fmt.Errorf("exact plan approval required")
	}
	if p.Gates.Unsafe && !consent.Unsafe {
		return fmt.Errorf("Unsafe acknowledgment required")
	}
	if p.Gates.Desktop && !consent.Desktop {
		return fmt.Errorf("desktop acknowledgment required")
	}
	if p.Gates.Backup && !consent.Backup {
		return fmt.Errorf("backup replacement acknowledgment required")
	}
	if _, err := e.Preview(ctx, p); err != nil {
		return err
	}
	if err := os.MkdirAll(e.config.State, 0700); err != nil {
		return err
	}
	unlock, err := e.lock()
	if err != nil {
		return err
	}
	defer unlock()
	if _, err := os.Lstat(e.config.State + "/recovery.json"); err == nil {
		return fmt.Errorf("interrupted operation requires explicit recovery")
	} else if !os.IsNotExist(err) {
		return err
	}
	if _, err := e.Preview(ctx, p); err != nil {
		return err
	}
	j, err := e.materialize(ctx, p)
	if err != nil {
		return err
	}
	if err := replace(e.config.State, "recovery.json", jsonState(j)); err != nil {
		return err
	}
	for _, a := range j.Actions {
		if err := ctx.Err(); err != nil {
			return err
		}
		if digest(a.Before) == digest(a.After) {
			continue
		}
		r, _ := e.root(a.Root)
		live, err := readFile(r, a.Path)
		if err != nil {
			return err
		}
		if digest(live) != digest(a.Before) {
			return fmt.Errorf("target changed during apply; inspect exact recovery: %s", a.Path)
		}
		if a.BackupPath != "" {
			backup, err := readFile(r, a.BackupPath)
			if err != nil {
				return err
			}
			if digest(backup) != digest(a.BackupBefore) {
				return fmt.Errorf("backup changed during apply; inspect exact recovery")
			}
			if err := replace(r, a.BackupPath, a.Before); err != nil {
				return err
			}
		}
		if err := replace(r, a.Path, a.After); err != nil {
			return err
		}
	}
	return replace(e.config.State, "recovery.json", FileState{})
}

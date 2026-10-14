package native

import (
	"context"
	"encoding/json"
	"fmt"
	"path/filepath"
	"sort"
	"strings"
)

func mergeSelection(old Entry, s Selection) Entry {
	x := s.Entry
	if x.Source == "" {
		x.Source = old.Source
	}
	if x.Destination == "" {
		x.Destination = old.Destination
	}
	if x.OS == nil {
		x.OS = old.OS
	}
	if x.Portability == "" {
		x.Portability = old.Portability
	}
	if x.Impact == "" {
		x.Impact = old.Impact
	}
	if x.Machines == nil {
		x.Machines = old.Machines
	}
	if x.Removals == nil {
		x.Removals = old.Removals
	}
	if x.Integrations == nil {
		x.Integrations = old.Integrations
	}
	if x.GitKeys == nil {
		x.GitKeys = old.GitKeys
	}
	if x.GitSource == "" {
		x.GitSource = old.GitSource
	}
	if x.GitValues == nil {
		x.GitValues = old.GitValues
	}
	if x.Template == "" {
		x.Template = old.Template
	}
	if x.Tools == nil {
		x.Tools = old.Tools
	}
	if x.Warnings == nil {
		x.Warnings = old.Warnings
	}
	if x.Requires == nil {
		x.Requires = old.Requires
	}
	if x.Notices == nil {
		x.Notices = old.Notices
	}
	x.LogicalID = old.LogicalID
	return x
}
func (e *Engine) planSource(ctx context.Context, p Plan, c Contract, m Manifest, a Adoption) (Plan, error) {
	r := p.Request
	p.Direction = "home → repository"
	if len(r.Metadata) > 0 || len(r.Profiles) > 0 {
		return p, fmt.Errorf("source changes require exact Track selections")
	}
	if r.Mode == "untrack" {
		if r.Entries == nil || len(r.Selections) > 0 {
			return p, fmt.Errorf("untrack requires exact ids")
		}
		seen := map[string]bool{}
		for _, id := range r.Entries {
			x, ok := m.Entries[id]
			if !ok || seen[id] {
				return p, fmt.Errorf("unknown/duplicate untrack id")
			}
			seen[id] = true
			p.Selected[id] = x
			p.Gates.Unsafe = p.Gates.Unsafe || x.Impact == "Unsafe"
			p.Gates.Desktop = p.Gates.Desktop || len(x.Warnings) > 0 || has(x.Integrations, "desktop")
			delete(m.Entries, id)
			logical := x.LogicalID
			if logical == "" {
				for name := range m.Tracking {
					for _, system := range []string{"linux", "darwin"} {
						if id == name+"@"+system {
							logical = name
						}
					}
				}
			}
			if logical != "" {
				l := m.Tracking[logical]
				for _, system := range x.OS {
					delete(l.Destinations, system)
					delete(l.Sources, system)
					delete(l.OSMetadata, system)
				}
				if len(l.Destinations) == 0 {
					delete(m.Tracking, logical)
				} else {
					m.Tracking[logical] = l
				}
			}
			for name, ids := range m.Profiles {
				filtered := []string{}
				for _, v := range ids {
					if v != id {
						filtered = append(filtered, v)
					}
				}
				m.Profiles[name] = filtered
			}
			// Removing enrollment never deletes live target or repository bytes.
			// Retained source may be referenced elsewhere or contain unrelated work.
		}
	} else {
		if len(r.Selections) == 0 || r.Entries != nil {
			return p, fmt.Errorf("Track requires exact file selections")
		}
		seen := map[string]bool{}
		for _, s := range r.Selections {
			if err := relative(s.Path); err != nil {
				return p, err
			}
			if seen[s.Path] {
				return p, fmt.Errorf("duplicate Track selection")
			}
			seen[s.Path] = true
			oldLogical := m.Tracking[s.Logical]
			oldLogicalSystems := sortedKeys(oldLogical.Sources)
			id := s.Path
			old := Entry{}
			for existing, x := range m.Entries {
				if x.Destination == s.Path && has(x.OS, a.Machine.OS) {
					id = existing
					old = x
					break
				}
			}
			x := mergeSelection(old, s)
			importKeys := x.GitKeys
			if has(old.Integrations, "git-config") {
				if s.GitKeys != nil {
					if len(s.GitKeys) == 0 {
						return p, fmt.Errorf("bounded Git selection must be nonempty")
					}
					for _, key := range s.GitKeys {
						if !has(old.GitKeys, key) {
							return p, fmt.Errorf("Git selection exceeds existing bounded fields")
						}
					}
				}
				x.GitKeys = old.GitKeys
				if x.GitValues == nil && old.GitSource != "" {
					values, err := e.gitEntryValues(ctx, old)
					if err != nil {
						return p, err
					}
					x.GitValues = values
				}
			}
			if s.Logical != "" {
				if m.Version != 2 || !machineID.MatchString(s.Logical) {
					return p, fmt.Errorf("logical Track requires manifest v2 and valid id")
				}
				if len(s.Destinations) == 0 {
					return p, fmt.Errorf("logical destinations required")
				}
				id = s.Logical + "@" + a.Machine.OS
				x = Entry{Source: filepath.Join(c.HomeSource, s.Path), Destination: s.Destinations[a.Machine.OS], OS: []string{a.Machine.OS}, Portability: "managed-shared", Impact: "Safe", Machines: s.Machines, LogicalID: s.Logical}
				for _, layer := range []map[string]any{s.Common, s.Machine[a.Machine.ID], s.OSMetadata[a.Machine.OS]} {
					var err error
					x, err = logicalLayer(x, layer)
					if err != nil {
						return p, err
					}
				}
				if m.Tracking == nil {
					m.Tracking = map[string]Logical{}
				}
				l := m.Tracking[s.Logical]
				if s.Common == nil {
					s.Common = l.Common
				}
				if s.Machine == nil {
					s.Machine = l.Machine
				}
				if s.OSMetadata == nil {
					s.OSMetadata = l.OSMetadata
				}
				if s.Machines == nil {
					x.Machines = l.Machines
				}
				if l.Sources == nil {
					l.Sources = map[string]string{}
				}
				for system, destination := range l.Destinations {
					if s.Destinations[system] != destination {
						return p, fmt.Errorf("existing logical mapping needs a metadata Track plan")
					}
				}
				for system := range s.Destinations {
					if _, exists := l.Sources[system]; !exists {
						l.Sources[system] = filepath.Join(c.HomeSource, ".dot-native", s.Logical+"@"+system)
					}
				}
				x.Source = l.Sources[a.Machine.OS]
				l.Destinations = s.Destinations
				l.Common = s.Common
				l.Machine = s.Machine
				l.OSMetadata = s.OSMetadata
				l.Machines = x.Machines
				m.Tracking[s.Logical] = l
				var err error
				x, err = compileLogical(s.Logical, l, a.Machine.OS, a.Machine.ID)
				if err != nil {
					return p, err
				}
			}
			if x.Source == "" {
				x.Source = filepath.Join(c.HomeSource, s.Path)
			}
			if x.Destination == "" {
				x.Destination = s.Path
			}
			if x.Machines == nil {
				x.Machines = &Assignment{Include: []string{"all"}, Exclude: []string{}}
			}
			if x.Destination != s.Path {
				return p, fmt.Errorf("Track mapping must name the exact selected target")
			}
			if err := e.validateEntry(x); err != nil {
				return p, err
			}
			if !within(filepath.Join(e.config.Source, x.Source), filepath.Join(e.config.Source, c.HomeSource)) {
				return p, fmt.Errorf("Track source outside home boundary")
			}
			if has(x.Integrations, "git-config") {
				values, err := e.readGitValues(ctx, e.config.Home, s.Path, importKeys)
				if err != nil {
					return p, err
				}
				if x.GitValues == nil {
					x.GitValues = map[string]string{}
				}
				for k, v := range values {
					x.GitValues[k] = v
				}
				x.GitSource = ""
				x.Template = ""
			} else {
				sdata, err := readFile(e.config.Home, s.Path)
				if err != nil {
					return p, err
				}
				if !sdata.Exists {
					return p, fmt.Errorf("missing exact Track file")
				}
				if x.Portability == "managed-templated" {
					if x.Template == "" {
						return p, fmt.Errorf("Track requires explicit secret-free template")
					}
					sdata.Data = []byte(x.Template)
					if _, err := e.renderTemplate(x, sdata); err != nil {
						return p, err
					}
				}
				if strings.HasPrefix(filepath.Base(x.Source), "run_") || strings.HasPrefix(filepath.Base(x.Source), "modify_") {
					return p, fmt.Errorf("executable private source is forbidden")
				}
				if err := e.addAction(&p, "source", x.Source, sdata); err != nil {
					return p, err
				}
			}
			m.Entries[id] = x
			p.Selected[id] = x
			if s.Logical != "" {
				l := m.Tracking[s.Logical]
				l.Machines = x.Machines
				m.Tracking[s.Logical] = l
				for _, system := range sortedKeys(l.Destinations) {
					canonical, err := compileLogical(s.Logical, l, system, "")
					if err != nil {
						return p, err
					}
					resolved, err := compileLogical(s.Logical, l, system, a.Machine.ID)
					if err != nil {
						return p, err
					}
					physical := s.Logical + "@" + system
					m.Entries[physical] = canonical
					p.Selected[physical] = resolved
					if system != a.Machine.OS && !has(oldLogicalSystems, system) {
						data, err := readFile(e.config.Home, s.Path)
						if err != nil {
							return p, err
						}
						if canonical.Portability == "managed-templated" {
							if canonical.Template == "" {
								return p, fmt.Errorf("each templated OS mapping requires explicit template")
							}
							data.Data = []byte(canonical.Template)
						}
						if err = e.addAction(&p, "source", canonical.Source, data); err != nil {
							return p, err
						}
					}
					if !has(m.Profiles[r.Profile], physical) {
						m.Profiles[r.Profile] = append(m.Profiles[r.Profile], physical)
					}
					p.Gates.Unsafe = p.Gates.Unsafe || resolved.Impact == "Unsafe"
					p.Gates.Desktop = p.Gates.Desktop || len(resolved.Warnings) > 0 || has(resolved.Integrations, "desktop")
				}
			}
			if x.Impact == "Unsafe" {
				p.Gates.Unsafe = true
			}
			if len(x.Warnings) > 0 || has(x.Integrations, "desktop") {
				p.Gates.Desktop = true
			}
			if !has(m.Profiles[r.Profile], id) {
				m.Profiles[r.Profile] = append(m.Profiles[r.Profile], id)
				sort.Strings(m.Profiles[r.Profile])
			}
		}
	}
	if err := e.validateManifest(c, m); err != nil {
		return p, err
	}
	if err := e.addAction(&p, "source", c.Manifest, jsonState(m)); err != nil {
		return p, err
	}
	p.ID = planID(p)
	return p, nil
}
func logicalLayer(x Entry, layer map[string]any) (Entry, error) {
	for key := range layer {
		if !has([]string{"portability", "impact", "removals", "tools", "warnings", "integrations", "requires", "notices", "git_keys", "git_values", "template"}, key) {
			return x, fmt.Errorf("unknown logical capability %q", key)
		}
	}
	b, _ := json.Marshal(x)
	var m map[string]any
	_ = json.Unmarshal(b, &m)
	for k, v := range layer {
		m[k] = v
	}
	b, _ = json.Marshal(m)
	err := Decode(b, &x)
	return x, err
}

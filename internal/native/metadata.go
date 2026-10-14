package native

import (
	"encoding/json"
	"fmt"
)

// Metadata is the logical-file Track editor: repository-side policy only, never
// an implicit adoption or target move. New OS enrollment requires an exact Track
// snapshot; changing an existing destination still requires subsequent Adopt.
func (e *Engine) planMetadata(p Plan, c Contract, m Manifest, a Adoption) (Plan, error) {
	p.Direction = "Track metadata → repository"
	r := p.Request
	if m.Version != 2 || len(r.Metadata) == 0 || r.Entries != nil || len(r.Selections) > 0 || len(r.Profiles) > 0 {
		return p, fmt.Errorf("logical metadata Track requires exact definitions")
	}
	for _, id := range sortedKeys(r.Metadata) {
		old, ok := m.Tracking[id]
		if !ok {
			return p, fmt.Errorf("unknown logical file")
		}
		change := r.Metadata[id]
		for k := range change {
			if !has([]string{"destinations", "common", "machine", "os_metadata", "machines"}, k) {
				return p, fmt.Errorf("unknown logical Track field %s", k)
			}
		}
		b, _ := json.Marshal(old)
		var fields map[string]any
		_ = json.Unmarshal(b, &fields)
		for k, v := range change {
			fields[k] = v
		}
		b, _ = json.Marshal(fields)
		var next Logical
		if err := Decode(b, &next); err != nil {
			return p, err
		}
		sources := map[string]string{}
		for system := range next.Destinations {
			source, ok := old.Sources[system]
			if !ok {
				return p, fmt.Errorf("new OS mappings require exact Track snapshot")
			}
			sources[system] = source
		}
		next.Sources = sources
		for system := range old.Destinations {
			physical := id + "@" + system
			before, err := compileLogical(id, old, system, a.Machine.ID)
			if err != nil {
				return p, err
			}
			p.Selected[physical] = before
			delete(m.Entries, physical)
			p.Gates.Unsafe = p.Gates.Unsafe || before.Impact == "Unsafe"
			p.Gates.Desktop = p.Gates.Desktop || len(before.Warnings) > 0 || has(before.Integrations, "desktop")
		}
		for system := range next.Destinations {
			canonical, err := compileLogical(id, next, system, "")
			if err != nil {
				return p, err
			}
			physical := id + "@" + system
			m.Entries[physical] = canonical
			resolved, err := compileLogical(id, next, system, a.Machine.ID)
			if err != nil {
				return p, err
			}
			p.Selected[physical] = resolved
			p.Gates.Unsafe = p.Gates.Unsafe || resolved.Impact == "Unsafe"
			p.Gates.Desktop = p.Gates.Desktop || len(resolved.Warnings) > 0 || has(resolved.Integrations, "desktop")
		}
		m.Tracking[id] = next
	}
	for profile, ids := range m.Profiles {
		filtered := []string{}
		for _, id := range ids {
			if _, ok := m.Entries[id]; ok {
				filtered = append(filtered, id)
			}
		}
		m.Profiles[profile] = filtered
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

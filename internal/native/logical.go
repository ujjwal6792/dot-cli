package native

import (
	"fmt"
	"strings"
)

func compileLogical(id string, l Logical, system, machine string) (Entry, error) {
	x := Entry{Source: l.Sources[system], Destination: l.Destinations[system], OS: []string{system}, Portability: "managed-shared", Impact: "Safe", Machines: l.Machines, LogicalID: id}
	for _, layer := range []map[string]any{l.Common, l.Machine[machine], l.OSMetadata[system]} {
		var err error
		x, err = logicalLayer(x, layer)
		if err != nil {
			return x, err
		}
	}
	return x, nil
}
func resolve(m Manifest, machine Machine) (Manifest, error) {
	if declared, ok := m.Machines[machine.ID]; ok && declared.OS != machine.OS {
		return m, fmt.Errorf("source catalog conflicts with bound machine OS")
	}
	entries := map[string]Entry{}
	for id, x := range m.Entries {
		entries[id] = x
	}
	m.Entries = entries
	for id, l := range m.Tracking {
		for system := range l.Destinations {
			x, err := compileLogical(id, l, system, machine.ID)
			if err != nil {
				return m, err
			}
			m.Entries[id+"@"+system] = x
		}
	}
	return m, nil
}
func (e *Engine) validateLogical(c Contract, m Manifest) error {
	for id, l := range m.Tracking {
		if id == "" || strings.ContainsAny(id, "\x00\r\n/@") || len(l.Destinations) == 0 || len(l.Destinations) != len(l.Sources) {
			return fmt.Errorf("invalid logical source/destination mappings")
		}
		if err := validateAssignment(l.Machines); err != nil {
			return err
		}
		for machine := range l.Machine {
			if !machineID.MatchString(machine) {
				return fmt.Errorf("invalid logical machine override")
			}
		}
		for system := range l.OSMetadata {
			if _, ok := l.Destinations[system]; !ok {
				return fmt.Errorf("unknown logical OS override")
			}
		}
		for system := range l.Destinations {
			if system != "linux" && system != "darwin" {
				return fmt.Errorf("unknown logical OS")
			}
			if _, ok := l.Sources[system]; !ok {
				return fmt.Errorf("missing logical OS source")
			}
			canonical, err := compileLogical(id, l, system, "")
			if err != nil {
				return err
			}
			if err = e.validateEntry(canonical); err != nil {
				return err
			}
			actual, ok := m.Entries[id+"@"+system]
			if !ok {
				return fmt.Errorf("missing physical logical entry")
			}
			actual.LogicalID = id
			// Legacy list restrictions lived on the logical definition, not the flat entry.
			if actual.Machines == nil {
				actual.Machines = l.Machines
			}
			if digest(actual) != digest(canonical) {
				return fmt.Errorf("logical/physical metadata conflict")
			}
			for machine := range l.Machine {
				x, err := compileLogical(id, l, system, machine)
				if err != nil {
					return err
				}
				if err = e.validateEntry(x); err != nil {
					return err
				}
			}
		}
	}
	return nil
}

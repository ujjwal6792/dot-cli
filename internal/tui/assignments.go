package tui

import (
	tea "charm.land/bubbletea/v2"
	"fmt"
)

// Assignment edits are intent, never authorization. Adopt only changes local
// selections/customs; changing shared assignment requires a reviewed Track plan.
func (m model) retargetCurrent() (tea.Model, tea.Cmd) {
	i := m.current()
	if m.mode != "track" {
		m.message = "Shared assignments require Track (2); Space deselects locally at Adopt."
		return m, nil
	}
	if i < 0 || m.entries[i].Kind != "file" {
		m.message = "m retargets one exact file, not a folder."
		return m, nil
	}
	entry := m.entries[i]
	if entry.Status != "eligible" && entry.Status != "managed" && !(entry.Status == "restricted" && boundedGitConfig(entry.Metadata)) {
		m.message = "Entry is not eligible for assignment."
		return m, nil
	}
	selection, ok := m.basket[entry.Path]
	if !ok {
		selection = cloneMetadata(entry.Metadata)
		selection["path"] = entry.Path
	} else {
		selection = cloneMetadata(selection)
	}
	if entry.LogicalID != "" {
		assigned := selection["machines"]
		selection = cloneMetadata(entry.Tracking)
		delete(selection, "sources")
		selection["path"], selection["logical"] = entry.Path, entry.LogicalID
		if assigned != nil {
			selection["machines"] = assigned
		}
	}
	choices := []string{"all"}
	if id := entryMachineScope(m); id != "" && id != "all" {
		choices = append(choices, id)
	}
	current := "all"
	if assigned, ok := selection["machines"].(map[string]any); ok {
		members := metadataList(assigned["include"])
		if len(members) == 1 {
			current = members[0]
		}
	}
	next := 0
	for index, choice := range choices {
		if choice == current {
			next = (index + 1) % len(choices)
			break
		}
	}
	selection["machines"] = map[string]any{"include": []string{choices[next]}, "exclude": []string{}}
	m.basket[entry.Path] = selection
	m.entries[i].Metadata = cloneMetadata(entry.Metadata)
	m.entries[i].Metadata["machines"] = selection["machines"]
	m.logicalRequest = nil
	m.generation++
	m.invalidatePlan()
	m.message = fmt.Sprintf("Assignment: %s · Track preview/approval required; nothing written.", assignmentLabel(choices[next]))
	return m, m.highlightCurrent()
}

// entryMachineScope constrains retargeting to All machines ↔ this machine
// only. Broader scopes never cycle here; shared changes need a reviewed
// Track plan.
func entryMachineScope(m model) string {
	if m.currentMachine.ID != "" {
		return m.currentMachine.ID
	}
	if len(m.machines) == 1 {
		return m.machines[0]
	}
	return ""
}

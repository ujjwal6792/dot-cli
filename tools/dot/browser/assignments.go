package browser

import (
	tea "charm.land/bubbletea/v2"
	"fmt"
)

// Assignment edits are intent, never authorization. Adopt only changes local
// selections/customs; changing shared assignment requires a reviewed Track plan.
func (m model) retargetCurrent() (tea.Model, tea.Cmd) {
	i := m.current()
	if m.mode != "track" {
		m.message = "Shared assignments require Track (Shift+Tab); Space deselects locally at Adopt."
		return m, nil
	}
	if i < 0 || m.entries[i].Kind != "file" {
		m.message = "Tab retargets one exact file, not a folder."
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
	choices := append([]string{"all"}, m.machines...)
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
	m.planFresh = false
	m.message = fmt.Sprintf("Machines: %s · Track preview/approval required; nothing written.", choices[next])
	return m, m.highlightCurrent()
}

package tui

import (
	"fmt"
	"sort"
	"strings"
)

// Directory projections keep the underlying full paths/IDs intact. They never
// broaden selection or consult HOME to infer which descendants are tracked.
func navigationParents(entries []Entry) []Entry {
	known := make(map[string]bool, len(entries))
	for _, e := range entries {
		known[e.Path] = true
	}
	for _, e := range entries {
		for p := parentPath(e.Path); p != ""; p = parentPath(p) {
			if !known[p] {
				entries = append(entries, Entry{Path: p, Kind: "directory", Status: "navigation"})
				known[p] = true
			}
		}
	}
	return entries
}

func trackedFile(e Entry) bool {
	return e.Kind == "file" && e.Status == "managed" && e.OSStatus != "unsupported"
}

func (m model) trackedBrowser() bool {
	return m.returning && m.mode == "track" && m.trackedOnly
}

func (m model) untrackedEntry(e Entry) bool {
	if m.mode != "track" {
		return false
	}
	if e.Kind != "directory" {
		return !trackedFile(e)
	}
	for _, child := range m.entries {
		if trackedFile(child) && strings.HasPrefix(child.Path, e.Path+"/") {
			return false
		}
	}
	return true
}

func (m model) fileRows(dir, query string) []int {
	trackedOnly := m.trackedBrowser() || m.mode != "track"
	allowed := make(map[string]bool, len(m.entries))
	for _, e := range m.entries {
		if trackedFile(e) {
			allowed[e.Path] = true
			for p := parentPath(e.Path); p != ""; p = parentPath(p) {
				allowed[p] = true
			}
		}
	}
	query = strings.ToLower(query)
	var rows []int
	for i, e := range m.entries {
		if e.OSStatus == "unsupported" || !m.hidden && (hiddenPath(e.Path) || e.Status == "hidden") {
			continue
		}
		if trackedOnly && !allowed[e.Path] {
			continue
		}
		if query != "" {
			if !strings.Contains(strings.ToLower(e.Path), query) {
				continue
			}
		} else if parentPath(e.Path) != dir {
			continue
		}
		rows = append(rows, i)
	}
	sort.SliceStable(rows, func(i, j int) bool {
		a, b := m.entries[rows[i]], m.entries[rows[j]]
		if query == "" && (a.Kind == "directory") != (b.Kind == "directory") {
			return a.Kind == "directory"
		}
		return a.Path < b.Path
	})
	return rows
}

func (m *model) toggleTrackedFolder(path string) {
	children := []Entry{}
	allSelected, skipped := true, 0
	for _, e := range m.entries {
		if !trackedFile(e) || !strings.HasPrefix(e.Path, path+"/") {
			continue
		}
		if !completeTrackMetadata(e.Metadata) {
			skipped++
			continue
		}
		children = append(children, e)
		if _, ok := m.basket[e.Path]; !ok {
			allSelected = false
		}
	}
	for _, e := range children {
		if allSelected {
			delete(m.basket, e.Path)
		} else {
			item := cloneMetadata(e.Metadata)
			item["path"] = e.Path
			m.basket[e.Path] = item
		}
	}
	m.invalidatePlan()
	m.generation++
	m.message = fmt.Sprintf("%d selected · tracked files only; untracked files remain unselected.", len(m.basket))
	if skipped > 0 {
		m.message += fmt.Sprintf(" %d need individual metadata/template review (e).", skipped)
	}
}

func dashboardRows(files []DashboardEntry, dir, query string, hidden bool) []DashboardEntry {
	query = strings.ToLower(query)
	folders := map[string]DashboardEntry{}
	var out []DashboardEntry
	for _, e := range files {
		if !hidden && hiddenPath(e.Path) {
			continue
		}
		if query != "" {
			if strings.Contains(strings.ToLower(e.Path+" "+e.Dest+" "+e.State), query) {
				out = append(out, e)
			}
			continue
		}
		rel := e.Path
		if dir != "" {
			if !strings.HasPrefix(rel, dir+"/") {
				continue
			}
			rel = strings.TrimPrefix(rel, dir+"/")
		}
		name, _, nested := strings.Cut(rel, "/")
		if !nested {
			out = append(out, e)
			continue
		}
		path := name
		if dir != "" {
			path = dir + "/" + name
		}
		folder := folders[path]
		folder.Path, folder.Dest, folder.Kind = path, path, "directory"
		folder.Tracked++
		switch e.State {
		case "changed", "error":
			folder.Changed++
		case "missing":
			folder.Missing++
		}
		folders[path] = folder
	}
	for _, folder := range folders {
		out = append(out, folder)
	}
	sort.SliceStable(out, func(i, j int) bool {
		if query == "" && (out[i].Kind == "directory") != (out[j].Kind == "directory") {
			return out[i].Kind == "directory"
		}
		return out[i].Path < out[j].Path
	})
	return out
}

func folderSummary(tracked, changed, missing int) string {
	text := fmt.Sprintf("%d tracked", tracked)
	if changed > 0 {
		text += fmt.Sprintf(" · %d changed", changed)
	}
	if missing > 0 {
		text += fmt.Sprintf(" · %d missing", missing)
	}
	return text
}

func (m model) fileFolderSummary(path string) string {
	tracked, changed, missing := m.fileFolderStats(path)
	return folderSummary(tracked, changed, missing)
}

func (m model) fileFolderStats(path string) (tracked, changed, missing int) {
	for _, e := range m.entries {
		if !trackedFile(e) || !strings.HasPrefix(e.Path, path+"/") {
			continue
		}
		tracked++
		switch e.TargetStatus {
		case "changed", "error":
			changed++
		case "missing":
			missing++
		}
	}
	return
}

func browserPath(dir string) string {
	if dir == "" {
		return "~"
	}
	return "~/" + safeTerminal(dir)
}

func browserName(path, query string) string {
	if query != "" {
		return safeTerminal(path)
	}
	return safeTerminal(pathBase(path))
}

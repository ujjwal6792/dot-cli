package tui

import (
	"context"
	"fmt"
	"time"

	tea "charm.land/bubbletea/v2"
)

// BrowserEngine is optional; scriptable inventory remains complete. Engines
// that implement it can populate the interactive HOME tree on expansion.
type BrowserEngine interface {
	BrowserInventory(context.Context, string) (Inventory, error)
	BrowseDirectory(context.Context, string) ([]Entry, error)
}

type directoryMsg struct {
	generation uint64
	path       string
	entries    []Entry
	err        error
}

// Explicit search builds a complete index once; opening Track never walks all
// of HOME. Keystrokes filter the loaded tree while Enter searches all folders.
func (m *model) searchAllFolders() tea.Cmd {
	if m.trackedBrowser() {
		return m.highlightCurrent()
	}
	if _, ok := m.engine.(BrowserEngine); !ok || m.mode != "track" || m.search.Value() == "" {
		return m.highlightCurrent()
	}
	m.cancelPreview()
	m.inventoryGeneration++
	m.loading = true
	m.message = "Searching all HOME folders…"
	return func() tea.Msg {
		value, err := m.engine.InventoryMode(m.setupCtx(), m.mode)
		return inventoryMsg{m.inventoryGeneration, m.mode, value, err}
	}
}

func (m *model) expandDirectory(index int) tea.Cmd {
	path := m.entries[index].Path
	if m.returning {
		m.browserDirs[m.currentTab()] = path
		m.cursor = 0
		m.search.SetValue("")
		m.searching = false
		m.search.Blur()
	}
	m.entries[index].Open = true
	m.filter()
	return m.loadBrowserDirectory(path)
}

func (m *model) loadBrowserDirectory(path string) tea.Cmd {
	e, ok := m.engine.(BrowserEngine)
	if !ok || m.mode != "track" || m.trackedBrowser() || m.dirLoaded[path] || m.dirPending[path] {
		return m.highlightCurrent()
	}
	if m.dirPending == nil {
		m.dirPending = map[string]bool{}
	}
	m.dirPending[path] = true
	m.message = "Loading " + path + "/…"
	generation, ctx := m.inventoryGeneration, m.setupCtx()
	return func() tea.Msg {
		entries, err := e.BrowseDirectory(ctx, path)
		return directoryMsg{generation, path, entries, err}
	}
}

func (m model) directoryResult(msg directoryMsg) (tea.Model, tea.Cmd) {
	if msg.generation != m.inventoryGeneration || m.mode != "track" {
		return m, nil
	}
	delete(m.dirPending, msg.path)
	if msg.err != nil {
		if m.returning && m.browserDirs[m.currentTab()] == msg.path {
			m.collapseDirectory()
		}
		m.message = fmt.Sprintf("Couldn't open %s: %s · l retries", msg.path, safeTerminal(msg.err.Error()))
		return m, nil
	}
	focus := ""
	if i := m.current(); i >= 0 {
		focus = m.entries[i].Path
	}
	known := make(map[string]bool, len(m.entries))
	for _, e := range m.entries {
		known[e.Path] = true
	}
	for _, e := range msg.entries {
		if !known[e.Path] {
			m.entries = append(m.entries, e)
			known[e.Path] = true
		}
	}
	if m.dirLoaded == nil {
		m.dirLoaded = map[string]bool{}
	}
	m.dirLoaded[msg.path] = true
	m.filter()
	for row, i := range m.visible {
		if m.entries[i].Path == focus {
			m.cursor = row
			break
		}
	}
	if cached, ok := m.invCache[m.mode]; ok {
		cached.Entries = m.entries
		m.invCache[m.mode] = cached
	}
	m.message = ""
	return m, m.highlightCurrent()
}

type previewRequestMsg struct {
	generation uint64
	ctx        context.Context
	mode, path string
	request    map[string]any
}

func (m *model) cancelPreview() {
	if m.previewCancel != nil {
		m.previewCancel()
		m.previewCancel = nil
	}
	m.previewGeneration++
	m.previewLoading = false
}

func (m *model) queuePreview(mode, path string, request map[string]any) tea.Cmd {
	m.cancelPreview()
	ctx, cancel := context.WithCancel(m.setupCtx())
	m.previewCancel = cancel
	m.highlightPath = path
	m.highlight = HighlightResult{}
	m.previewLoading = true
	m.previewViewport.SetContent("")
	m.previewViewport.GotoTop()
	msg := previewRequestMsg{m.previewGeneration, ctx, mode, path, request}
	return tea.Tick(35*time.Millisecond, func(time.Time) tea.Msg { return msg })
}

func (m model) startPreview(msg previewRequestMsg) (tea.Model, tea.Cmd) {
	if msg.generation != m.previewGeneration || msg.ctx.Err() != nil {
		return m, nil
	}
	return m, func() tea.Msg {
		result, err := m.engine.Highlight(msg.ctx, msg.mode, msg.path, msg.request)
		return highlightMsg{msg.generation, result, err}
	}
}

func pagedCursor(cursor, count, page int, key string) int {
	switch key {
	case "home":
		return 0
	case "end":
		return max(0, count-1)
	case "pgup":
		return max(0, cursor-max(1, page))
	case "pgdown":
		return min(max(0, count-1), cursor+max(1, page))
	}
	return cursor
}

func (m *model) pageFiles(key string) {
	m.cursor = pagedCursor(m.cursor, len(m.visible), m.fileListLimit(), key)
}

func (m *model) collapseDirectory() {
	if m.returning {
		path := m.browserDirs[m.currentTab()]
		m.browserDirs[m.currentTab()] = parentPath(path)
		m.filter()
		for row, index := range m.visible {
			if m.entries[index].Path == path {
				m.cursor = row
				break
			}
		}
		return
	}
	i := m.current()
	if i < 0 {
		return
	}
	if m.entries[i].Kind == "directory" && m.entries[i].Open {
		m.entries[i].Open = false
		m.filter()
		return
	}
	parent := parentPath(m.entries[i].Path)
	for row, index := range m.visible {
		if m.entries[index].Path == parent {
			m.cursor = row
			m.entries[index].Open = false
			m.filter()
			return
		}
	}
}

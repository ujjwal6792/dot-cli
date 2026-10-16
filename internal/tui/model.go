package tui

import (
	"context"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"regexp"
	"sort"
	"strings"

	"charm.land/bubbles/v2/textarea"
	"charm.land/bubbles/v2/textinput"
	"charm.land/bubbles/v2/viewport"
	"charm.land/bubbletea/v2"
	"charm.land/huh/v2"
	"charm.land/lipgloss/v2"
)

type Engine interface {
	InventoryMode(context.Context, string) (Inventory, error)
	Snapshot(context.Context, string) (SnapshotResult, error)
	Highlight(context.Context, string, string, map[string]any) (HighlightResult, error)
	Plan(context.Context, map[string]any) (PlanResult, error)
	Preview(context.Context, any) (PlanResult, error)
	Apply(context.Context, any, string, bool, bool, bool) error
}

type PlanResult struct {
	Plan       map[string]any `json:"plan"`
	Direction  string         `json:"direction"`
	Diffs      []Diff         `json:"diffs"`
	Unsafe     bool           `json:"-"`
	Desktop    bool           `json:"-"`
	Backup     bool           `json:"-"`
	GatesKnown bool           `json:"-"`
	Notices    []string       `json:"notices,omitempty"`
	Summary    []string       `json:"summary,omitempty"`
	Review     string         `json:"review,omitempty"`
}
type Diff struct {
	Path    string `json:"path"`
	Status  string `json:"status"`
	Summary string `json:"summary"`
	Text    string `json:"text"`
}

type model struct {
	returning                                   bool
	returningChoice                             int
	returningOperation                          string
	dashboard                                   *DashboardData
	dashboardErr                                error
	dashboardLoading                            bool
	packageView                                 bool
	packages                                    PackageResult
	packageStage, packageManager, packageReview string
	packagePlan                                 map[string]any
	packageCursor                               int
	packageGeneration                           uint64
	packageEditOriginal, packageEditInitial     string

	engine                           Engine
	entries                          []Entry
	profiles                         []Profile
	machines                         []string
	inventoryReady                   bool
	inventoryErr                     error
	presetPending                    bool
	visible                          []int
	cursor                           int
	basket                           map[string]map[string]any
	metadataEdits                    map[string]map[string]any
	mode                             string
	profile                          string
	selectedProfiles                 []string
	adoptTouched                     bool
	hidden                           bool
	search                           textinput.Model
	searching                        bool
	width, height                    int
	message                          string
	selected                         *PlanResult
	unsafeAck, desktopAck, backupAck bool
	stage                            string
	viewport                         viewport.Model
	previewViewport                  viewport.Model
	templateInput                    textarea.Model
	loading                          bool
	planFresh                        bool
	generation                       uint64
	inventoryGeneration              uint64
	err                              error
	rounds                           bool
	setupChoice                      int
	presetChoice                     int
	confirmChoice                    int
	editField                        int
	editPath                         string
	editValue                        map[string]any
	editOriginal                     map[string]any
	editTemplate                     bool
	previewGeneration                uint64
	highlight                        HighlightResult
	highlightPath                    string
	currentOS                        string
	currentMachine                   Machine
	logicalRequest                   map[string]any
	initMode                         bool
	sourceChoice                     int
	sourceInput                      textinput.Model
	sourceAction, sourceBackup       string
	sourcePlan                       map[string]any
	sourceError                      string
	machineForm                      *huh.Form
	machineOS                        string
	dependencies                     DependencyResult
	dependenciesNext                 string
	trackerPlan                      map[string]any
	trackerReview                    string
	trackerName, trackerEmail        textinput.Model
	trackerField                     int
	trackerEditing                   bool
	setupContext                     context.Context
	setupCancel                      context.CancelFunc
	activeTab                        int
	syncChoice                       int
	tabGeneration                    uint64
	settingsArmed                    bool
	settingsSearch                   textinput.Model
	settingsReady                    bool
	settingsFocus                    int
	settingsAdvanced                 bool
	settingsCheckState               string
	help                             bool
	helpOffset                       int
	tabCursors                       [7]int
	listOffsets                      [7]int
	invCache                         map[string]Inventory
	dirLoaded                        map[string]bool
	dirPending                       map[string]bool
	previewCancel                    context.CancelFunc
	previewLoading                   bool
}

type inventoryMsg struct {
	generation uint64
	mode       string
	value      Inventory
	err        error
}
type planMsg struct {
	generation uint64
	value      PlanResult
	err        error
}
type previewMsg struct {
	generation uint64
	original   PlanResult
	preview    PlanResult
	err        error
}
type applyMsg struct{ err error }
type snapshotMsg struct {
	generation uint64
	folder     string
	result     SnapshotResult
	err        error
}
type highlightMsg struct {
	generation uint64
	result     HighlightResult
	err        error
}

func NewModel(engine Engine, rounds bool, mode string) tea.Model {
	input := textinput.New()
	input.Placeholder = "filter paths"
	input.Prompt = "/ "
	if mode != "adopt" {
		mode = "track"
	}
	stage := ""
	if rounds {
		stage = "setup"
	}
	templateEditor := textarea.New()
	templateEditor.Placeholder = "Secret-free template"
	templateEditor.SetWidth(72)
	templateEditor.SetHeight(12)
	return model{engine: engine, basket: map[string]map[string]any{}, metadataEdits: map[string]map[string]any{}, mode: mode, profile: "default", search: input, templateInput: templateEditor, rounds: rounds, stage: stage, hidden: true, viewport: viewport.New(viewport.WithWidth(76), viewport.WithHeight(15)), previewViewport: viewport.New(viewport.WithWidth(50), viewport.WithHeight(14)), width: 80, height: 24, message: "Nothing selected. Browsing is not authorization."}
}

func NewInitModel(engine Engine) tea.Model {
	return NewInitModelContext(context.Background(), engine)
}

// NewInitModelContext binds explicit setup effects to the app's cancellation.
func NewInitModelContext(ctx context.Context, engine Engine) tea.Model {
	ctx, cancel := context.WithCancel(ctx)
	input := textinput.New()
	input.Placeholder = "filter paths"
	input.Prompt = "/ "
	templateEditor := textarea.New()
	templateEditor.Placeholder = "Secret-free template"
	templateEditor.SetWidth(72)
	templateEditor.SetHeight(12)
	return model{engine: engine, setupContext: ctx, setupCancel: cancel, initMode: true, stage: "welcome", basket: map[string]map[string]any{}, metadataEdits: map[string]map[string]any{}, mode: "track", profile: "default", search: input, templateInput: templateEditor, hidden: true, viewport: viewport.New(viewport.WithWidth(76), viewport.WithHeight(15)), previewViewport: viewport.New(viewport.WithWidth(50), viewport.WithHeight(14)), width: 80, height: 24, message: "Nothing selected. Browsing is not authorization."}
}
func (m model) Init() tea.Cmd {
	if m.stage == "setup" || m.initMode {
		return nil
	}
	if m.returning {
		return m.dashboardCommand()
	}
	return m.loadInventory
}
func (m model) loadInventory() tea.Msg {
	var value Inventory
	var err error
	if e, ok := m.engine.(BrowserEngine); ok {
		value, err = e.BrowserInventory(m.setupCtx(), m.mode)
	} else {
		value, err = m.engine.InventoryMode(m.setupCtx(), m.mode)
	}
	return inventoryMsg{m.inventoryGeneration, m.mode, value, err}
}

// Opening presets is navigation only, including when requested before inventory
// finishes. Never infer an empty preset catalog from an unfinished response.
func (m *model) openPresets() {
	m.presetPending = false
	if len(m.profiles) == 0 {
		m.message = "Engine did not provide named presets."
		return
	}
	m.message = ""
	m.stage = "preset"
	m.presetChoice = 0
}

func (m model) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch value := msg.(type) {
	case directoryMsg:
		return m.directoryResult(value)
	case previewRequestMsg:
		return m.startPreview(value)
	case setupMsg:
		return m.setupResult(value)
	case returningMsg:
		if !value.remote && value.generation != m.tabGeneration {
			return m, nil
		}
		m.loading = false
		m.err = value.err
		if value.remote {
			m.stage = "returning-home"
			m.returningChoice = 0
			m.message = "Explicit " + m.returningOperation + " completed; no files applied or committed."
			if value.err != nil {
				m.message = value.err.Error()
			}
			if cmd := m.dashboardCommand(); cmd != nil {
				m.dashboardLoading = true
				m.dashboardErr = nil
				return m, cmd
			}
		} else {
			m.stage = "returning-inspect"
			m.resizeTextViewport()
			m.viewport.SetContent(safeTerminal(value.text))
			m.viewport.GotoTop()
		}
		return m, nil
	case dashboardMsg:
		if value.generation != m.tabGeneration {
			return m, nil
		}
		m.dashboardLoading = false
		m.dashboardErr = value.err
		if value.err == nil {
			m.dashboard = value.data
			if value.data != nil && value.data.Machine.ID != "" {
				m.currentMachine = value.data.Machine
			}
			if m.stage == "returning-home" && m.activeTab == TabStatus {
				return m, m.statusHighlight()
			}
		} else {
			m.dashboard = nil
		}
		return m, nil
	case settingsDepsMsg:
		return m.settingsResultDeps(value)
	case settingsCheckMsg:
		return m.settingsResultCheck(value)
	case machineMsg:
		return m.machineResult(value)
	case sourceMsg:
		if m.returning && m.currentTab() == TabSettings && m.stage == "returning-settings" {
			return m.settingsSourceResult(value)
		}
		return m.sourceResult(value)
	case tea.PasteMsg:
		if m.stage == "returning-settings" && m.settingsSearch.Focused() {
			var cmd tea.Cmd
			m.settingsSearch, cmd = m.settingsSearch.Update(value)
			m.refreshTabViewport()
			m.viewport.GotoTop()
			return m, cmd
		}
		if m.initMode && m.stage == "tracker-edit" && !m.loading {
			return m.updateTrackerInput(value)
		}
		if m.initMode && m.stage == "tracker-confirm" && m.trackerEditing && !m.loading {
			return m.updateTrackerInput(value)
		}
		if m.stage == "returning-settings" && m.settingsSearch.Focused() {
			var cmd tea.Cmd
			m.settingsSearch, cmd = m.settingsSearch.Update(msg)
			return m, cmd
		}
		if m.initMode && m.stage == "machine" && !m.loading {
			return m.updateMachineForm(value)
		}
		if m.initMode && m.stage == "source-url" && !m.loading {
			var cmd tea.Cmd
			m.sourceInput, cmd = m.sourceInput.Update(value)
			return m, cmd
		}
	case packagesMsg:
		if !m.packageView || value.generation != m.packageGeneration {
			return m, nil
		}
		m.loading = false
		m.err = value.err
		if value.err != nil {
			m.message = value.err.Error()
		}
		if value.action == "apply" {
			m.packageStage = ""
			m.packagePlan = nil
			if value.err != nil {
				m.message = "Installation rejected or interrupted; refresh and review a new plan before retrying."
				return m, nil
			}
			m.message = "Reviewed installation completed; refreshing package status."
			return m, m.packageCommand("status", nil)
		}
		if value.err == nil {
			if value.action == "plan" {
				m.packagePlan = value.result.Plan
				m.packageReview = value.result.Review
				m.packageStage = "review"
				m.viewport.GotoTop()
				m.message = "Review is not consent."
			} else {
				if value.action == "save" || value.action == "import" {
					for manager, source := range value.result.Sources {
						m.packages.Sources[manager] = source
					}
					m.packageStage = ""
					m.packagePlan = nil
					m.message = "Private source saved with backup. Nothing installed/uninstalled; r refresh status."
					return m, nil
				}
				m.packages = value.result
				m.packageCursor = min(m.packageCursor, max(0, len(m.packages.Rows)-1))
				m.packageStage = ""
				m.packagePlan = nil
				if m.packageManager == "" {
					names := m.packageManagers()
					if len(names) > 0 {
						m.packageManager = names[0]
					}
				}
			}
		}
		return m, nil
	case inventoryMsg:
		if m.packageView {
			return m, nil
		}
		if value.generation != m.inventoryGeneration || value.mode != m.mode {
			return m, nil
		}
		m.loading = false
		m.err = value.err
		m.inventoryErr = value.err
		m.inventoryReady = value.err == nil
		if value.err == nil {
			m.currentOS = value.value.OS
			m.currentMachine = value.value.Machine
			m.entries = value.value.Entries
			m.dirLoaded, m.dirPending = map[string]bool{}, map[string]bool{}
			m.profiles = value.value.Profiles
			m.machines = value.value.Machines
			if m.invCache == nil {
				m.invCache = map[string]Inventory{}
			}
			m.invCache[value.mode] = value.value
			m.filter()
			if m.presetPending {
				m.openPresets()
			}
			cmd := m.highlightCurrent()
			return m, cmd
		}
		m.presetPending = false
	case snapshotMsg:
		if value.generation != m.generation {
			return m, nil
		}
		m.loading = false
		m.err = value.err
		if value.err == nil {
			byPath := map[string]Entry{}
			for _, entry := range m.entries {
				byPath[entry.Path] = entry
			}
			for _, path := range value.result.Paths {
				metadata := map[string]any{"path": path}
				if entry, ok := byPath[path]; ok {
					for k, v := range entry.Metadata {
						metadata[k] = v
					}
				}
				m.basket[path] = metadata
			}
			m.message = fmt.Sprintf("Snapshotted %d eligible exact files; %d ineligible skipped. Future files remain unselected.", len(value.result.Paths), value.result.Skipped)
		}
	case highlightMsg:
		if value.generation != m.previewGeneration {
			return m, nil
		}
		m.previewLoading = false
		m.err = value.err
		if value.err == nil {
			m.highlight = value.result
			m.resizePreview()
			m.previewViewport.SetContent(m.renderHighlight())
			m.previewViewport.GotoTop()
		} else {
			m.highlight = HighlightResult{}
			m.previewViewport.SetContent(m.renderHighlight())
		}
	case planMsg:
		if value.generation != m.generation {
			return m, nil
		}
		m.loading = false
		m.err = value.err
		if value.err == nil {
			m.loading = true
			return m, func() tea.Msg {
				preview, err := m.engine.Preview(m.setupCtx(), value.value.Plan)
				return previewMsg{value.generation, value.value, preview, err}
			}
		}
	case previewMsg:
		if value.generation != m.generation {
			return m, nil
		}
		m.loading = false
		m.err = value.err
		if value.err == nil {
			reviewed := value.preview
			reviewed.Plan = value.original.Plan
			if reviewed.Direction == "" {
				reviewed.Direction = value.original.Direction
			}
			if len(reviewed.Diffs) == 0 && len(value.original.Diffs) > 0 {
				reviewed.Diffs = value.original.Diffs
			}
			if reviewed.Review == "" {
				reviewed.Review = value.original.Review
			}
			reviewed.Unsafe = value.original.Unsafe
			reviewed.Desktop = value.original.Desktop
			reviewed.Backup = value.original.Backup
			reviewed.GatesKnown = value.original.GatesKnown
			m.selected = &reviewed
			m.planFresh = true
			m.stage = "review"
			m.message = "Review the exact plan. Nothing applied."
			m.refreshReviewViewport()
			m.viewport.GotoTop()
		}
	case applyMsg:
		m.loading = false
		m.err = value.err
		m.invCache = nil
		m.cancelPreview()
		if value.err == nil {
			m.stage = "done"
			m.message = "Applied reviewed plan."
		} else {
			m.stage = "review"
			m.planFresh = false
			m.unsafeAck, m.desktopAck, m.backupAck = false, false, false
			m.message = "Apply rejected: " + safeTerminal(value.err.Error()) + ". Refresh the exact plan before another approval."
		}
	case tea.WindowSizeMsg:
		m.width, m.height = value.Width, value.Height
		if m.settingsReady {
			m.settingsSearch.SetWidth(max(1, value.Width-7))
		}
		if m.machineForm != nil {
			m.machineForm.WithWidth(max(20, value.Width-8))
		}
		m.viewport.SetWidth(max(20, value.Width-4))
		m.viewport.SetHeight(max(4, value.Height-9))
		if m.initMode {
			switch m.stage {
			case "dependencies":
				m.viewport.SetHeight(m.depHeight())
			case "tracker-confirm":
				m.viewport.SetHeight(m.trackerHeight())
			case "source-confirm":
				m.viewport.SetHeight(m.sourceConfirmHeight())
			}
		}
		m.viewport.SoftWrap = true
		m.resizePreview()
		m.resizeTextViewport()
		if m.stage == "review" {
			m.refreshReviewViewport()
		}

		m.templateInput.SetWidth(max(20, value.Width-8))
		m.templateInput.SetHeight(max(4, value.Height-10))
		if m.returning {
			m.templateInput.SetHeight(max(3, m.workspaceHeight()-6))
		}
	case tea.KeyPressMsg:
		if m.help {
			switch value.String() {
			case "esc", "?":
				m.help = false
			case "j", "J", "down":
				m.helpOffset = min(15, m.helpOffset+1)
			case "k", "K", "up":
				m.helpOffset = max(0, m.helpOffset-1)
			case "q", "ctrl+c":
				return m, tea.Quit
			}
			return m, nil
		}
		if value.String() == "?" && !m.searching && !m.initMode && m.stage != "metadata" && m.stage != "template" && m.stage != "logical" && !m.packageView && !(m.stage == "returning-settings" && m.settingsSearch.Focused()) {
			m.help, m.helpOffset = true, 0
			return m, nil
		}
		// Bubbles/Huh system-clipboard shortcuts invoke platform helpers. Dot
		// instead accepts the terminal's bracketed PasteMsg, keeping runtime
		// dependencies confined to Git/chezmoi and explicitly approved managers.
		if value.String() == "ctrl+v" || value.String() == "ctrl+shift+c" {
			m.message = "Use your terminal paste/copy shortcut; dot does not run clipboard helpers."
			return m, nil
		}
		if value.String() == "ctrl+c" {
			if m.setupCancel != nil {
				m.setupCancel()
			}
			return m, tea.Interrupt
		}
		if m.returning {
			if next, cmd, ok := m.returningTabKey(value); ok {
				return next, cmd
			}
			if strings.HasPrefix(m.stage, "returning-") {
				return m.updateReturning(value)
			}
		}
		if m.returning && (m.stage == "done" && value.String() == "enter" || m.stage == "" && !m.searching && value.String() == "esc") {
			m.resetReturning()
			if cmd := m.dashboardCommand(); cmd != nil {
				m.dashboardLoading = true
				m.dashboardErr = nil
				return m, cmd
			}
			return m, nil
		}
		if m.packageView {
			return m.updatePackages(value)
		}
		if m.initMode {
			return m.updateInit(value)
		}
		if value.String() == "P" && m.stage == "" && !m.searching {
			m.packageView = true
			m.packageStage = ""
			m.selected = nil
			m.generation++
			m.previewGeneration++
			m.inventoryGeneration++
			m.message = "Navigation never installs."
			return m, m.packageCommand("status", nil)
		}
		if m.stage == "done" && m.rounds && value.String() == "enter" {
			m.stage = "setup"
			m.setupChoice = 0
			m.basket = map[string]map[string]any{}
			m.metadataEdits = map[string]map[string]any{}
			m.logicalRequest = nil
			m.selected = nil
			m.unsafeAck, m.desktopAck, m.backupAck = false, false, false
			m.profile = "default"
			m.selectedProfiles = nil
			m.adoptTouched = false
			m.search.SetValue("")
			m.searching = false
			m.cursor = 0
			m.hidden = true
			m.generation++
			m.previewGeneration++
			m.message = "Nothing selected. Browsing is not authorization."
			return m, nil
		}
		if m.stage == "setup" {
			switch value.String() {
			case "j", "down":
				m.setupChoice = min(2, m.setupChoice+1)
			case "k", "up":
				m.setupChoice = max(0, m.setupChoice-1)
			case "q", "esc":
				return m, tea.Quit
			case "enter":
				if m.setupChoice == 0 {
					return m, tea.Quit
				}
				if m.setupChoice == 1 {
					m.mode = "track"
				} else {
					m.mode = "adopt"
				}
				m.stage = ""
				m.loading = true
				m.inventoryGeneration++
				m.inventoryReady, m.presetPending = false, false
				return m, m.loadInventory
			}
			return m, nil
		}
		if m.stage == "preset" {
			switch value.String() {
			case "j", "down":
				m.presetChoice = min(len(m.profiles)-1, m.presetChoice+1)
			case "k", "up":
				m.presetChoice = max(0, m.presetChoice-1)
			case "esc", "q":
				m.stage = ""
			case "enter":
				if len(m.profiles) > 0 {
					profile := m.profiles[m.presetChoice]
					m.logicalRequest = nil
					m.generation++
					for _, entry := range m.entries {
						for _, id := range profile.Entries {
							if entry.ID == id && entry.Status == "managed" {
								item := map[string]any{"path": entry.Path, "id": entry.ID}
								m.basket[entry.Path] = item
							}
						}
					}
					m.profile = profile.Name
					found := false
					for _, name := range m.selectedProfiles {
						if name == profile.Name {
							found = true
						}
					}
					if !found {
						m.selectedProfiles = append(m.selectedProfiles, profile.Name)
					}
					sort.Strings(m.selectedProfiles)
					m.stage = ""
					m.invalidatePlan()
					m.message = fmt.Sprintf("Composed profiles %s (%d exact entries); Space deselects locally; no apply consent.", strings.Join(m.selectedProfiles, " + "), len(m.basket))
				}
			}
			return m, nil
		}
		if m.stage == "metadata" {
			return m.updateMetadata(value)
		}
		if m.stage == "logical" {
			if value.String() == "esc" {
				m.stage = ""
				return m, nil
			}
			if value.String() == "ctrl+s" || tea.Key(value).Code == 19 {
				request, err := uniqueJSONRequest(m.templateInput.Value())
				if err != nil {
					m.message = "Invalid JSON: " + safeTerminal(err.Error()) + "; keep editing or Esc to cancel."
					return m, nil
				}
				mode, _ := request["mode"].(string)
				if mode != "metadata" && mode != "track" {
					m.message = "Only Dotfiles Track or metadata requests belong in this editor."
					return m, nil
				}
				m.logicalRequest = request
				m.basket = map[string]map[string]any{}
				m.invalidatePlan()
				m.generation++
				m.stage = ""
				m.message = "Logical metadata request prepared; p reviews exact source/scope changes. No consent given."
				return m, nil
			}
			var cmd tea.Cmd
			m.templateInput, cmd = m.templateInput.Update(value)
			return m, cmd
		}
		if m.stage == "template" {
			if value.String() == "esc" {
				m.stage = "metadata"
				return m, nil
			}
			if value.String() == "ctrl+s" || tea.Key(value).Code == 19 {
				m.editValue["template"] = m.templateInput.Value()
				m.stage = "metadata"
				return m, nil
			}
			var cmd tea.Cmd
			m.templateInput, cmd = m.templateInput.Update(value)
			return m, cmd
		}
		if m.searching {
			return m.updateSearch(value)
		}
		if value.String() == "ctrl+c" {
			return m, tea.Quit
		}
		if m.stage == "review" {
			return m.updateReview(value)
		}
		if m.stage == "confirm" {
			return m.updateConfirm(value)
		}
		if value.String() == "q" {
			if m.setupCancel != nil {
				m.setupCancel()
			}
			return m, tea.Quit
		}
		switch value.String() {
		case "j", "down":
			if m.cursor < len(m.visible)-1 {
				m.cursor++
			}
			m.generation++
			cmd := m.highlightCurrent()
			return m, cmd
		case "k", "up":
			if m.cursor > 0 {
				m.cursor--
			}
			m.generation++
			cmd := m.highlightCurrent()
			return m, cmd
		case "pgdown", "pgup", "home", "end":
			m.pageFiles(value.String())
			return m, m.highlightCurrent()
		case "h", "left":
			m.collapseDirectory()
			return m, m.highlightCurrent()
		case "l", "right", "enter":
			if i := m.current(); i >= 0 && m.entries[i].Kind == "directory" {
				cmd := m.expandDirectory(i)
				return m, cmd
			}
		case " ", "space":
			cmd := m.toggleCurrent()
			if cmd != nil {
				return m, cmd
			}
			cmd = m.highlightCurrent()
			return m, cmd
		case "e":
			if i := m.current(); m.mode != "untrack" && i >= 0 && (m.entries[i].Kind == "file" || m.entries[i].Kind == "directory") {
				m.beginEdit(m.entries[i])
			}
		case "M":
			if i := m.current(); i >= 0 && m.entries[i].Kind == "file" && m.mode != "untrack" {
				entry := m.entries[i]
				request := map[string]any{"version": 1, "mode": "metadata", "profile": m.profile}
				if entry.LogicalID != "" {
					definition := cloneMetadata(entry.Tracking)
					delete(definition, "sources")
					request["metadata"] = map[string]any{entry.LogicalID: definition}
				} else {
					request["mode"] = "track"
					request["selections"] = []map[string]any{{"path": entry.Path, "logical": fmt.Sprintf("file_%x", sha256.Sum256([]byte(entry.Path))), "destinations": map[string]string{m.currentOS: entry.Path}, "common": map[string]any{"portability": "managed-shared", "impact": "Safe"}, "machine": map[string]any{}, "os_metadata": map[string]any{}, "machines": map[string]any{"include": []string{"all"}, "exclude": []string{}}}}
				}
				body, _ := json.Marshal(request)
				m.templateInput.SetValue(string(body))
				m.templateInput.Focus()
				m.message = ""
				m.stage = "logical"
			}
		case "s":
			if m.mode == "adopt" {
				if !m.inventoryReady {
					m.presetPending = true
					m.message = "Loading named presets; no files selected or approved."
				} else {
					m.openPresets()
				}
			}
		case "m", "tab":
			return m.retargetCurrent()
		case "shift+tab":
			m.cancelPreview()
			m.dirPending = map[string]bool{}
			m.selectedProfiles = nil
			m.adoptTouched = false
			m.logicalRequest = nil
			if m.mode == "track" {
				m.mode = "adopt"
			} else {
				m.mode = "track"
			}
			if m.returning {
				if m.mode == "track" {
					m.activeTab = TabTrack
				} else {
					m.activeTab = TabAdopt
				}
			}
			m.basket = map[string]map[string]any{}
			m.metadataEdits = map[string]map[string]any{}
			m.invalidatePlan()
			m.generation++
			m.inventoryGeneration++
			m.inventoryReady, m.presetPending = false, false
			m.cursor = 0
			m.filter()
			m.loading = true
			return m, m.loadInventory
		case "u":
			m.cancelPreview()
			m.dirPending = map[string]bool{}
			m.selectedProfiles = nil
			m.adoptTouched = false
			m.logicalRequest = nil
			if m.mode == "untrack" {
				m.mode = "track"
			} else {
				m.mode = "untrack"
			}
			if m.returning {
				if m.mode == "untrack" {
					m.activeTab = TabUntrack
				} else {
					m.activeTab = TabTrack
				}
			}
			m.basket = map[string]map[string]any{}
			m.metadataEdits = map[string]map[string]any{}
			m.invalidatePlan()
			m.generation++
			m.inventoryGeneration++
			m.inventoryReady, m.presetPending = false, false
			m.cursor = 0
			m.loading = true
			m.message = "Stop tracking reviews private source/scope only; live files unchanged."
			return m, m.loadInventory
		case "H":
			m.hidden = !m.hidden
			m.filter()
			cmd := m.highlightCurrent()
			return m, cmd
		case "J":
			m.resizePreview()
			m.previewViewport.ScrollDown(1)
		case "K":
			m.resizePreview()
			m.previewViewport.ScrollUp(1)
		case "/":
			m.searching = true
			m.search.SetValue("")
			m.search.Focus()
		case "p":
			cmd := m.plan()
			return m, cmd
		case "r":
			m.cancelPreview()
			m.generation++
			m.inventoryGeneration++
			m.inventoryReady, m.presetPending = false, false
			m.invCache = nil
			m.loading = true
			m.message = ""
			return m, m.loadInventory
		case "esc":
			m.search.SetValue("")
			m.filter()
		}
	}
	if m.initMode && m.stage == "machine" && !m.loading {
		return m.updateMachineForm(msg)
	}
	if m.initMode && m.stage == "tracker-edit" && !m.loading {
		return m.updateTrackerInput(msg)
	}
	if m.initMode && m.stage == "tracker-confirm" && m.trackerEditing && !m.loading {
		return m.updateTrackerInput(msg)
	}
	if m.initMode && m.stage == "source-url" && !m.loading {
		var cmd tea.Cmd
		m.sourceInput, cmd = m.sourceInput.Update(msg)
		return m, cmd
	}
	return m, nil
}

func (m *model) updateSearch(key tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	if key.String() == "esc" {
		m.searching = false
		m.search.Blur()
		m.search.SetValue("")
		m.filter()
		cmd := m.highlightCurrent()
		return m, cmd
	}
	if key.String() == "enter" {
		m.searching = false
		m.search.Blur()
		m.filter()
		cmd := m.searchAllFolders()
		return m, cmd
	}
	var cmd tea.Cmd
	m.search, cmd = m.search.Update(key)
	m.filter()
	previewCmd := m.highlightCurrent()
	return m, tea.Batch(cmd, previewCmd)
}

func (m *model) current() int {
	if m.cursor < 0 || m.cursor >= len(m.visible) {
		return -1
	}
	return m.visible[m.cursor]
}

func (m *model) highlightCurrent() tea.Cmd {
	m.keepFileCursorVisible()
	i := m.current()
	if i < 0 {
		m.cancelPreview()
		m.highlight = HighlightResult{}
		m.highlightPath = ""
		return nil
	}
	entry := m.entries[i]
	m.highlightPath = entry.Path
	var request map[string]any
	if selected, ok := m.basket[entry.Path]; ok && m.mode == "track" {
		selection := map[string]any{}
		for k, v := range selected {
			selection[k] = v
		}
		allowed := map[string]bool{"path": true, "destination": true, "os": true, "portability": true, "impact": true, "template": true, "tools": true, "removals": true, "warnings": true, "integrations": true, "requires": true, "notices": true, "git_keys": true, "machines": true}
		if selection["logical"] != nil {
			allowed = map[string]bool{"path": true, "logical": true, "destinations": true, "common": true, "machine": true, "os_metadata": true, "machines": true, "template": true}
		}
		for k := range selection {
			if !allowed[k] {
				delete(selection, k)
			}
		}
		request = map[string]any{"version": 1, "mode": "track", "profile": m.profile, "selections": []map[string]any{selection}}
	}
	if entry.Kind == "directory" {
		m.cancelPreview()
		m.highlightPath = entry.Path
		m.highlight = HighlightResult{Status: "directory", Summary: "Enter / l expands this folder · Space snapshots exact eligible files."}
		m.previewViewport.SetContent(m.renderHighlight())
		m.previewViewport.GotoTop()
		return nil
	}
	if m.stage != "review" {
		request = nil
	}
	return m.queuePreview(m.mode, entry.Path, request)
}

func (m *model) filter() {
	m.visible = m.visible[:0]
	byPath := make(map[string]Entry, len(m.entries))
	for _, e := range m.entries {
		byPath[e.Path] = e
	}
	query := strings.ToLower(m.search.Value())
	for i, e := range m.entries {
		if !m.hidden && hiddenPath(e.Path) {
			continue
		}
		if e.Status == "hidden" && !m.hidden {
			continue
		}
		if query != "" {
			if !strings.Contains(strings.ToLower(e.Path), query) {
				continue
			}
			m.visible = append(m.visible, i)
			continue
		}
		parent := parentPath(e.Path)
		for parent != "" {
			p, ok := byPath[parent]
			if ok && !p.Open {
				goto skip
			}
			parent = parentPath(parent)
		}
		m.visible = append(m.visible, i)
	skip:
	}
	sort.SliceStable(m.visible, func(i, j int) bool { return m.entries[m.visible[i]].Path < m.entries[m.visible[j]].Path })
	if m.cursor >= len(m.visible) {
		m.cursor = max(0, len(m.visible)-1)
	}
}

func hiddenPath(path string) bool {
	for _, part := range strings.Split(path, "/") {
		if strings.HasPrefix(part, ".") {
			return true
		}
	}
	return false
}

func pathBase(path string) string {
	if i := strings.LastIndex(path, "/"); i >= 0 {
		return path[i+1:]
	}
	return path
}
func parentPath(path string) string {
	if i := strings.LastIndex(path, "/"); i >= 0 {
		return path[:i]
	}
	return ""
}

// Applicability is resolved by the shared engine, including explicit-machine
// restrictions. Folder snapshots consume that result; navigation and individual
// metadata editing remain independent of this bulk-selection affordance.
func adoptFolderFile(entry Entry) bool {
	return entry.Kind == "file" && entry.Status == "managed" && entry.ID != "" && entry.OSStatus == "supported"
}

func (m *model) toggleCurrent() tea.Cmd {
	m.logicalRequest = nil
	i := m.current()
	if i < 0 {
		return nil
	}
	entry := m.entries[i]
	if entry.Kind == "directory" {
		if m.mode == "adopt" {
			m.adoptTouched = true
			// Freeze the currently inventoried file IDs, not a recursive path or
			// profile: later repository entries never acquire implicit consent.
			children := []Entry{}
			allSelected := true
			for _, child := range m.entries {
				if adoptFolderFile(child) && strings.HasPrefix(child.Path, entry.Path+"/") {
					children = append(children, child)
					if _, ok := m.basket[child.Path]; !ok {
						allSelected = false
					}
				}
			}
			for _, child := range children {
				if allSelected {
					delete(m.basket, child.Path)
				} else {
					item := cloneMetadata(child.Metadata)
					item["path"], item["id"] = child.Path, child.ID
					m.basket[child.Path] = item
				}
			}
			m.invalidatePlan()
			m.generation++
			return nil
		}
		if m.mode == "untrack" {
			entry.Open = !entry.Open
			m.entries[i] = entry
			m.filter()
			return nil
		}
		m.generation++
		generation := m.generation
		m.loading = true
		m.invalidatePlan()
		return func() tea.Msg {
			result, err := m.engine.Snapshot(m.setupCtx(), entry.Path)
			return snapshotMsg{generation, entry.Path, result, err}
		}
	}
	selectable := entry.Status == "eligible" || m.mode == "track" && entry.Status == "managed" || m.mode == "track" && entry.Status == "restricted" && boundedGitConfig(entry.Metadata) || (m.mode == "adopt" || m.mode == "untrack") && entry.Status == "managed" && entry.ID != ""
	if !selectable || entry.Kind != "file" {
		m.message = "Entry is not selectable: " + entry.Status
		return nil
	}
	if _, exists := m.basket[entry.Path]; exists {
		if m.mode == "adopt" {
			m.adoptTouched = true
		}
		delete(m.basket, entry.Path)
		m.invalidatePlan()
		m.generation++
		return nil
	}
	if m.mode == "adopt" || m.mode == "untrack" {
		if m.mode == "adopt" {
			m.adoptTouched = true
		}
		id := entry.ID
		item := map[string]any{"path": entry.Path, "id": id}
		for k, v := range entry.Metadata {
			item[k] = v
		}
		m.basket[entry.Path] = item
		m.invalidatePlan()
		m.generation++
		return nil
	}
	metadata := map[string]any{"path": entry.Path}
	for k, v := range entry.Metadata {
		metadata[k] = v
	}
	if !completeTrackMetadata(metadata) {
		m.beginEdit(entry)
		return nil
	}
	m.basket[entry.Path] = metadata
	m.invalidatePlan()
	m.generation++
	return nil
}

func completeTrackMetadata(v map[string]any) bool {
	if v["destination"] == "" || v["os"] == nil || v["portability"] == "" || v["impact"] == "" {
		return false
	}
	if v["portability"] == "managed-templated" && v["template"] == nil && !boundedGitConfig(v) {
		return false
	}
	if values, ok := stringList(v["integrations"]); ok {
		for _, integration := range values {
			if integration == "git-config" && v["git_keys"] == nil {
				return false
			}
		}
	}
	return true
}

func boundedGitConfig(v map[string]any) bool {
	if v["portability"] != "managed-templated" || v["git_keys"] == nil {
		return false
	}
	for _, integration := range metadataList(v["integrations"]) {
		if integration == "git-config" {
			return true
		}
	}
	return false
}

func metadataList(v any) []string {
	if values, ok := stringList(v); ok {
		return values
	}
	if value, ok := v.(string); ok {
		return strings.Split(value, ",")
	}
	return nil
}
func (m *model) beginEdit(entry Entry) {
	m.logicalRequest = nil
	m.editPath = entry.Path
	m.editValue = map[string]any{"path": entry.Path}
	for k, v := range entry.Metadata {
		m.editValue[k] = v
	}
	m.editOriginal = cloneMetadata(m.editValue)
	for _, field := range []string{"os", "git_keys", "removals", "tools", "warnings", "integrations", "requires", "notices"} {
		if values, ok := stringList(m.editValue[field]); ok {
			m.editValue[field] = strings.Join(values, ",")
		}
	}
	if m.editValue["destination"] == nil {
		m.editValue["destination"] = entry.Path
	}
	m.editField = 0
	m.stage = "metadata"
	m.message = ""
}

func (m *model) plan() tea.Cmd {
	if len(m.basket) == 0 && m.logicalRequest == nil && !(m.mode == "adopt" && (m.adoptTouched || len(m.selectedProfiles) > 0)) {
		m.message = "Select at least one exact eligible file."
		return nil
	}
	paths := make([]string, 0, len(m.basket))
	for p := range m.basket {
		paths = append(paths, p)
	}
	sort.Strings(paths)
	selections := make([]map[string]any, 0, len(paths))
	for _, p := range paths {
		selections = append(selections, cloneMetadata(m.basket[p]))
	}
	request := map[string]any{"version": 1, "mode": m.mode, "profile": m.profile}
	if m.mode == "adopt" && len(m.selectedProfiles) > 0 {
		request["profiles"] = append([]string{}, m.selectedProfiles...)
	}
	if m.mode == "track" {
		allowed := map[string]bool{"path": true, "destination": true, "os": true, "portability": true, "impact": true, "template": true, "tools": true, "removals": true, "warnings": true, "integrations": true, "requires": true, "notices": true, "git_keys": true, "machines": true}
		for _, selection := range selections {
			fields := allowed
			if selection["logical"] != nil {
				fields = map[string]bool{"path": true, "logical": true, "destinations": true, "common": true, "machine": true, "os_metadata": true, "machines": true, "template": true}
			}
			for key := range selection {
				if !fields[key] {
					delete(selection, key)
				}
			}
		}
		request["selections"] = selections
	} else {
		ids := make([]string, 0, len(paths))
		for _, p := range paths {
			id, _ := m.basket[p]["id"].(string)
			ids = append(ids, id)
		}
		sort.Strings(ids)
		request["entries"] = ids
		metadata := map[string]any{}
		for path, changes := range m.metadataEdits {
			id, _ := m.basket[path]["id"].(string)
			if id != "" {
				metadata[id] = changes
			}
		}
		if len(metadata) > 0 && m.mode == "adopt" {
			request["metadata"] = metadata
		}
	}
	if m.logicalRequest != nil {
		request = m.logicalRequest
	}
	m.generation++
	m.planFresh = false
	m.selected = nil
	m.stage = ""
	m.message = "Generating and freshness-checking the exact plan…"
	m.unsafeAck = false
	m.desktopAck = false
	m.backupAck = false
	generation := m.generation
	m.loading = true
	return func() tea.Msg {
		value, err := m.engine.Plan(m.setupCtx(), request)
		return planMsg{generation, value, err}
	}
}

func (m model) updateReview(key tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	switch key.String() {
	case "q", "esc":
		m.selected = nil
		m.stage = ""
		m.message = "Review canceled; nothing applied."
	case "j", "down":
		m.viewport.ScrollDown(1)
	case "k", "up":
		m.viewport.ScrollUp(1)
	case "p":
		cmd := m.plan()
		return m, cmd
	case "u":
		m.unsafeAck = !m.unsafeAck
	case "d":
		m.desktopAck = !m.desktopAck
	case "b":
		m.backupAck = !m.backupAck
	case "a":
		if m.selected == nil {
			return m, nil
		}
		if !m.planFresh {
			m.message = "This plan is no longer fresh. Press p to generate and review a new plan."
			return m, nil
		}
		r := m.selected
		if !r.GatesKnown {
			m.message = "Engine does not report required consent gates; apply is disabled."
			return m, nil
		}
		if r.Unsafe && !m.unsafeAck {
			m.message = "Unsafe acknowledgment required separately."
			return m, nil
		}
		if r.Desktop && !m.desktopAck {
			m.message = "Desktop acknowledgment required separately."
			return m, nil
		}
		if r.Backup && !m.backupAck {
			m.message = "Backup replacement acknowledgment required separately."
			return m, nil
		}
		m.stage = "confirm"
		m.confirmChoice = 0
	}
	return m, nil
}

var metadataChoices = map[string][]string{
	"portability":  {"managed-shared", "managed-local", "managed-templated", "managed-os-only"},
	"impact":       {"Safe", "Unsafe"},
	"os":           {"darwin", "linux", "darwin,linux"},
	"git_keys":     {"", "user.name", "user.email", "user.name,user.email"},
	"integrations": {"", "git-config", "gemini-skills", "claude-skills", "desktop"},
}

var metadataGuidance = map[string]string{
	"destination":  "Relative destination path under HOME (e.g. .bashrc).",
	"os":           "Target OS: darwin, linux, or darwin,linux (managed-os-only requires exactly 1 OS).",
	"portability":  "Portability tier: managed-shared, managed-local, managed-templated, or managed-os-only.",
	"impact":       "Impact level: Safe (non-destructive) or Unsafe (may overwrite/remove).",
	"template":     "Template body (managed-templated only). Press Ctrl+T to open multi-line editor.",
	"git_keys":     "Permitted Git config keys: user.name, user.email (requires managed-templated).",
	"removals":     "Comma-separated list of paths to remove before applying.",
	"tools":        "Comma-separated list of tool dependencies required for this entry.",
	"warnings":     "Comma-separated list of operational warnings.",
	"integrations": "Permitted integrations: git-config, gemini-skills, claude-skills, desktop.",
}

func (m model) updateMetadata(key tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	if key.String() == "esc" {
		m.stage = ""
		return m, nil
	}
	fields := []string{"destination", "os", "portability", "impact", "template", "git_keys", "removals", "tools", "warnings", "integrations"}
	if key.String() == "tab" || key.String() == "down" {
		m.editField = (m.editField + 1) % len(fields)
		m.message = ""
		return m, nil
	}
	if key.String() == "shift+tab" || key.String() == "up" {
		m.editField = (m.editField - 1 + len(fields)) % len(fields)
		m.message = ""
		return m, nil
	}
	field := fields[m.editField]
	if field == "template" && key.String() == "ctrl+t" {
		if current, ok := m.editValue["template"].(string); ok {
			m.templateInput.SetValue(current)
		}
		m.templateInput.Focus()
		m.stage = "template"
		return m, nil
	}
	switch key.String() {
	case "right", "l":
		if choices, ok := metadataChoices[field]; ok {
			cur := inputValue(m.editValue[field])
			idx := -1
			for i, c := range choices {
				if c == cur {
					idx = i
					break
				}
			}
			nextIdx := (idx + 1) % len(choices)
			m.editValue[field] = choices[nextIdx]
			m.message = ""
			return m, nil
		}
	case "left", "h":
		if choices, ok := metadataChoices[field]; ok {
			cur := inputValue(m.editValue[field])
			idx := -1
			for i, c := range choices {
				if c == cur {
					idx = i
					break
				}
			}
			prevIdx := (idx - 1 + len(choices)) % len(choices)
			m.editValue[field] = choices[prevIdx]
			m.message = ""
			return m, nil
		}
	case " ", "space":
		if choices, ok := metadataChoices[field]; ok {
			cur := inputValue(m.editValue[field])
			idx := -1
			for i, c := range choices {
				if c == cur {
					idx = i
					break
				}
			}
			nextIdx := (idx + 1) % len(choices)
			m.editValue[field] = choices[nextIdx]
			m.message = ""
			return m, nil
		}
		m.editValue[field] = inputValue(m.editValue[field]) + " "
		return m, nil
	case "enter":
		if m.mode == "track" && (m.editValue["destination"] == "" || m.editValue["os"] == nil || m.editValue["os"] == "" || m.editValue["portability"] == nil || m.editValue["portability"] == "" || m.editValue["impact"] == nil || m.editValue["impact"] == "") {
			m.message = "Destination, OS, portability, and impact are explicit required metadata."
			return m, nil
		}
		portability := inputValue(m.editValue["portability"])
		if portability != "" && !has([]string{"managed-shared", "managed-local", "managed-templated", "managed-os-only"}, portability) {
			m.message = fmt.Sprintf("Invalid portability %q (supported: managed-shared, managed-local, managed-templated, managed-os-only)", portability)
			return m, nil
		}
		impact := inputValue(m.editValue["impact"])
		if impact != "" && impact != "Safe" && impact != "Unsafe" {
			m.message = fmt.Sprintf("Invalid impact %q (supported: Safe, Unsafe)", impact)
			return m, nil
		}
		osVal := inputValue(m.editValue["os"])
		if osVal != "" {
			osList := strings.Split(osVal, ",")
			for _, o := range osList {
				o = strings.TrimSpace(o)
				if o != "darwin" && o != "linux" {
					m.message = fmt.Sprintf("Invalid OS %q (supported: darwin, linux)", o)
					return m, nil
				}
			}
			if portability == "managed-os-only" && len(osList) != 1 {
				m.message = "managed-os-only requires exactly one OS (darwin or linux)"
				return m, nil
			}
		}
		gitKeysVal := inputValue(m.editValue["git_keys"])
		if gitKeysVal != "" {
			for _, k := range strings.Split(gitKeysVal, ",") {
				k = strings.TrimSpace(k)
				if k != "" && k != "user.name" && k != "user.email" {
					m.message = fmt.Sprintf("Invalid git key %q (supported: user.name, user.email)", k)
					return m, nil
				}
			}
		}
		integrationsVal := inputValue(m.editValue["integrations"])
		if integrationsVal != "" {
			for _, ig := range strings.Split(integrationsVal, ",") {
				ig = strings.TrimSpace(ig)
				if ig != "" && !has([]string{"git-config", "gemini-skills", "claude-skills", "desktop"}, ig) {
					m.message = fmt.Sprintf("Invalid integration %q (supported: git-config, gemini-skills, claude-skills, desktop)", ig)
					return m, nil
				}
			}
		}
		value := cloneMetadata(m.editValue)
		for _, field := range []string{"os", "git_keys", "removals", "tools", "warnings", "integrations", "requires", "notices"} {
			if raw, ok := value[field].(string); ok {
				items := []string{}
				for _, item := range strings.Split(raw, ",") {
					if item = strings.TrimSpace(item); item != "" {
						items = append(items, item)
					}
				}
				value[field] = items
			}
		}
		if value["portability"] == "managed-templated" && !boundedGitConfig(value) {
			template, ok := value["template"].(string)
			if !ok || strings.TrimSpace(template) == "" {
				m.message = "Managed-templated Track requires an explicit secret-free template."
				return m, nil
			}
			if !strings.HasSuffix(template, "\n") {
				value["template"] = template + "\n"
			}
		}
		if m.mode == "adopt" {
			changes := map[string]any{}
			for _, name := range []string{"os", "impact", "removals", "tools", "warnings", "integrations", "requires", "notices", "git_keys"} {
				if fieldValue, ok := value[name]; ok && fmt.Sprint(fieldValue) != fmt.Sprint(m.editOriginal[name]) {
					changes[name] = fieldValue
				}
			}
			if len(changes) > 0 {
				m.metadataEdits[m.editPath] = changes
			} else {
				delete(m.metadataEdits, m.editPath)
			}
		} else if m.entriesByPath(m.editPath).Kind == "directory" {
			prefix := m.editPath + "/"
			for path := range m.basket {
				if strings.HasPrefix(path, prefix) {
					exact := cloneMetadata(value)
					exact["path"] = path
					if exact["destination"] == m.editPath {
						exact["destination"] = path
					}
					m.basket[path] = exact
				}
			}
		} else {
			m.basket[m.editPath] = value
		}
		m.stage = ""
		m.invalidatePlan()
		m.generation++
		m.message = "Exact selection metadata saved; preview is still required."
	case "backspace":
		if value, ok := m.editValue[field].(string); ok && len(value) > 0 {
			m.editValue[field] = value[:len(value)-1]
		}
	case "ctrl+u", "ctrl+U":
		m.editValue[field] = ""
	default:
		if tea.Key(key).Code == 21 {
			m.editValue[field] = ""
			return m, nil
		}
		if len(key.String()) == 1 {
			current := inputValue(m.editValue[field])
			if field == "git_keys" && current == strings.Join(metadataList(m.editOriginal[field]), ",") {
				current = ""
			}
			m.editValue[field] = current + key.String()
		}
	}
	return m, nil
}

func cloneMetadata(v map[string]any) map[string]any {
	out := map[string]any{}
	for k, x := range v {
		out[k] = x
	}
	return out
}

func inputValue(value any) string {
	if value == nil {
		return ""
	}
	return fmt.Sprint(value)
}

func has(list []string, s string) bool {
	for _, x := range list {
		if x == s {
			return true
		}
	}
	return false
}

func (m model) entriesByPath(path string) Entry {
	for _, entry := range m.entries {
		if entry.Path == path {
			return entry
		}
	}
	return Entry{}
}

func stringList(value any) ([]string, bool) {
	switch v := value.(type) {
	case []string:
		return v, true
	case []any:
		out := make([]string, 0, len(v))
		for _, item := range v {
			out = append(out, fmt.Sprint(item))
		}
		return out, true
	}
	return nil, false
}

func (m model) updateConfirm(key tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	if m.loading {
		return m, nil
	}
	switch key.String() {
	case "q", "esc":
		m.stage = "review"
		m.unsafeAck, m.desktopAck, m.backupAck = false, false, false
		m.message = "Final confirmation canceled; acknowledgments cleared."
	case "j", "down":
		m.confirmChoice = 1
	case "k", "up":
		m.confirmChoice = 0
	case "enter":
		if m.confirmChoice == 0 {
			m.stage = "review"
			m.unsafeAck, m.desktopAck, m.backupAck = false, false, false
			m.message = "Final confirmation canceled; acknowledgments cleared."
			return m, nil
		}
		r := m.selected
		id, _ := r.Plan["id"].(string)
		m.loading = true
		return m, func() tea.Msg {
			err := m.engine.Apply(m.setupCtx(), r.Plan, id, m.unsafeAck, m.desktopAck, m.backupAck)
			return applyMsg{err}
		}
	}
	return m, nil
}

var muted = lipgloss.NewStyle().Foreground(lipgloss.Color(AppTheme.Muted))
var strong = lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color(AppTheme.Text))
var ansiCSI = regexp.MustCompile("\\x1b\\[[0-?]*[ -/]*[@-~]")

func safeTerminal(value string) string {
	value = ansiCSI.ReplaceAllString(value, "")
	return strings.Map(func(r rune) rune {
		if (r < 0x20 && r != '\n' && r != '\t') || r == 0x7f {
			return -1
		}
		return r
	}, value)
}

func sanitizeTerminalView(value string) string {
	return strings.Map(func(r rune) rune {
		if (r < 0x20 && r != '\n' && r != '\t' && r != 0x1b) || r == 0x7f {
			return -1
		}
		return r
	}, value)
}

func fitHeight(value string, height int) string {
	lines := strings.Split(value, "\n")
	if len(lines) <= height {
		return value
	}
	if height <= 2 {
		return strings.Join(lines[:max(1, height)], "\n")
	}
	focusIdx := -1
	for idx, line := range lines {
		trimmed := strings.TrimSpace(line)
		if strings.HasPrefix(trimmed, ">") || strings.Contains(line, "> ") {
			focusIdx = idx
			break
		}
	}
	if focusIdx >= 0 {
		start := max(0, min(focusIdx-height+1, len(lines)-height))
		if focusIdx < start {
			start = focusIdx
		}
		end := min(len(lines), start+height)
		return strings.Join(lines[start:end], "\n")
	}
	return strings.Join(lines[:height], "\n")
}

// overlayStyle is the compact fallback for legacy inline flows and very small
// setup terminals. Workspaces and regular setup screens own bounded panels.
var overlayStyle = lipgloss.NewStyle().Padding(0, 2)

func (m model) renderOverlay(body string) string {
	w := m.width
	if w < 1 {
		w = 80
	}
	return overlayStyle.Width(w).Render(body)
}

func (m model) View() tea.View {
	height := m.height
	if height < 1 {
		height = 24
	}
	var content string
	if m.help {
		content = m.renderKeyHelp()
	} else {
		content = m.render()
	}
	content = sanitizeTerminalView(content)
	v := tea.NewView(fitHeight(content, height))
	v.AltScreen = true
	return v
}

func (m model) render() string {
	if m.initMode && (m.stage == "dependencies" || strings.HasPrefix(m.stage, "tracker-") || m.stage == "setup-incomplete") {
		return m.renderSetup()
	}
	if m.returning && strings.HasPrefix(m.stage, "returning-") {
		if m.hasStatusWorkspace() || m.stage == "returning-sync" || m.stage == "returning-settings" || m.stage == "returning-inspect" || m.stage == "returning-remote-confirm" {
			return m.renderReturning()
		}
		return m.renderOverlay(m.renderReturning())
	}
	if m.initMode && m.stage == "machine" {
		return m.renderMachine()
	}
	if m.initMode && strings.HasPrefix(m.stage, "source-") {
		return m.renderSource()
	}
	if m.packageView {
		return m.renderPackages()
	}
	if m.width < 1 {
		m.width = 80
	}
	if m.height < 1 {
		m.height = 24
	}
	if m.stage == "review" && m.selected != nil {
		return m.returningChrome(m.renderReview())
	}
	if m.stage == "metadata" {
		return m.returningChrome(m.renderMetadata())
	}
	if m.stage == "template" {
		return m.returningChrome("EDIT SECRET-FREE TEMPLATE\n" + m.templateInput.View() + "\n\nCtrl+S save · Esc cancel")
	}
	if m.stage == "logical" {
		return m.returningChrome("LOGICAL FILE METADATA · Dotfiles\nCommon → explicit machine → OS; per-OS destinations.\n" + m.templateInput.View() + "\n\nCtrl+S prepare request · Esc cancel · no writes until exact review/approval\n" + safeTerminal(m.message))
	}
	if m.stage == "confirm" {
		choices := []string{"Cancel", "Apply exact reviewed plan"}
		selected := m.confirmChoice
		return m.returningChrome(fmt.Sprintf("FINAL CONFIRMATION · %s\nPlan ID: %v\n\nUnsafe %t · Desktop %t · Backup replacement %t\n\n%s %s\n%s %s\n\nj/k choose · Enter confirm · Esc cancel", m.selected.Direction, m.selected.Plan["id"], m.unsafeAck, m.desktopAck, m.backupAck, map[bool]string{true: ">", false: " "}[selected == 0], choices[0], map[bool]string{true: ">", false: " "}[selected == 1], choices[1]))
	}
	if m.stage == "setup" {
		choices := []string{"Finish without further adoption", "Track: HOME → repository", "Adopt: repository → HOME"}
		var b strings.Builder
		b.WriteString("Optional configuration rounds\nInstallation is complete · nothing preselected\n\n")
		for i, c := range choices {
			mark := "  "
			if i == m.setupChoice {
				mark = "> "
			}
			fmt.Fprintf(&b, "%s%s\n", mark, c)
		}
		b.WriteString("\nj/k move · Enter select · q quit")
		return m.renderOverlay(b.String())
	}
	if m.stage == "preset" {
		var b strings.Builder
		b.WriteString("OPT-IN EXACT REPOSITORY PRESET · COMPOSE PROFILES\nUnion of flat lists · Space deselects locally after selection · no apply consent\n\n")
		for i, p := range m.profiles {
			mark := "  "
			if i == m.presetChoice {
				mark = "> "
			}
			fmt.Fprintf(&b, "%s%s · %d entries\n", mark, p.Name, len(p.Entries))
		}
		b.WriteString("\nj/k move · Enter select · Esc cancel")
		return m.returningChrome(b.String())
	}
	if m.stage == "welcome" && m.initMode {
		message := m.message
		if message == "Nothing selected. Browsing is not authorization." {
			// That default belongs to the file browser's selection basket;
			// on welcome nothing can be selected yet, so show no status.
			message = ""
		}
		if m.loading {
			message = "Checking source…"
		}
		body := "• Track — version exact files from HOME into your repository.\n• Adopt — apply repository files to this machine, only the scope you select.\n• Machine — a local id so shared files know where they belong.\n• Dependencies — Git and chezmoi only, installed with your Enter consent.\n• Tracker — a local record of this connection; never commits or pushes.\n\nThis wizard only connects repository and machine; adoption stays a separate decision.\n\ndot init --check is offline/read-only. Entering setup changes nothing."
		return m.initShell("Welcome", "Welcome to dot", "What dot does", body, "", "Enter continue · q quit", message)
	}
	if m.stage == "done" {
		if m.initMode {
			machine := "Machine identity saved locally."
			if m.currentMachine.ID != "" {
				machine = "Machine " + safeTerminal(m.currentMachine.ID) + " (" + safeTerminal(m.currentMachine.OS) + ") — identity saved locally."
			}
			body := "✓ Source connected — repository checked, contract valid.\n✓ " + machine + "\n✓ Dependencies verified — Git and chezmoi ready.\n✓ Tracker connected — local record, nothing pushed.\n\nAdoption scope unchanged — no files adopted, no Git settings changed."
			return m.initShell("Done", "Done", "Setup complete — summary", body, "", "Enter open returning menu · q quit", m.message)
		}
		if m.rounds {
			return "Applied reviewed plan.\nEnter next optional round · q quit"
		}
		return m.browserChrome("Applied reviewed plan.\n" + muted.Render("q quit"))
	}
	return m.renderFileWorkspace()
}

func (m model) emptyReason() (string, string) {
	if len(m.entries) == 0 {
		if m.inventoryErr != nil {
			return "Couldn't list files", "Fix the error below, then press r to rescan."
		}
		return "Empty repository", m.emptyGuidance()
	}
	if strings.TrimSpace(m.search.Value()) != "" {
		return fmt.Sprintf("No search matches for %q", m.search.Value()), ""
	}
	if !m.hidden {
		return "Hidden results (press H to show)", ""
	}
	return "No entry", ""
}

// emptyGuidance tells a user with no entries what to do next, per mode.
// Wording stays inside established product truth: track versions HOME files,
// adopt applies repository files, rescanning is a read-only reload.
func (m model) emptyGuidance() string {
	switch m.mode {
	case "adopt":
		return "Nothing to import yet.\nTrack files first, or connect a repository that has some."
	case "untrack":
		return "Nothing to stop tracking."
	default:
		return "No files to version yet.\nCreate files in HOME, then press r to rescan."
	}
}

func (m model) renderHighlight() string {
	direction := "HOME → REPOSITORY"
	if m.mode == "adopt" || m.returning && m.currentTab() == TabStatus {
		direction = "REPOSITORY → TARGET"
	}
	if m.highlight.Direction != "" {
		direction = strings.ToUpper(safeTerminal(m.highlight.Direction))
	}
	lines := []string{"PREVIEW · " + direction, safeTerminal(m.highlight.Status) + " · " + safeTerminal(m.highlight.Summary)}
	if m.highlight.Text != "" {
		lines = append(lines, safeTerminal(m.highlight.Text))
	} else {
		for _, diff := range m.highlight.Diffs {
			if diff.Path != m.highlightPath {
				continue
			}
			if diff.Summary != "" {
				lines = append(lines, safeTerminal(diff.Summary))
			}
			if diff.Text != "" {
				lines = append(lines, safeTerminal(diff.Text))
			}
		}
	}
	text := strings.Join(lines, "\n")
	var styled []string
	for _, line := range strings.Split(text, "\n") {
		color := AppTheme.Text
		switch {
		case strings.HasPrefix(line, "+++") || strings.HasPrefix(line, "---"):
			color = AppTheme.Muted
		case strings.HasPrefix(line, "+"):
			color = AppTheme.Success
		case strings.HasPrefix(line, "-"):
			color = AppTheme.Danger
		case strings.HasPrefix(line, "@@"):
			color = AppTheme.Accent
		}
		styled = append(styled, lipgloss.NewStyle().Foreground(lipgloss.Color(color)).Render(line))
	}
	return strings.Join(styled, "\n")
}

func (m model) reviewText() string {
	r := m.selected
	var b strings.Builder
	if r.Review != "" {
		b.WriteString(safeTerminal(r.Review) + "\n")
	}
	for _, d := range r.Diffs {
		fmt.Fprintf(&b, "\n%s · %s · %s\n%s\n", safeTerminal(d.Path), safeTerminal(d.Status), safeTerminal(d.Summary), safeTerminal(d.Text))
	}
	for _, notice := range r.Notices {
		fmt.Fprintf(&b, "\nNOTICE · %s\n", safeTerminal(notice))
	}
	for _, summary := range r.Summary {
		fmt.Fprintf(&b, "%s\n", safeTerminal(summary))
	}
	return b.String()
}

func (m *model) refreshReviewViewport() {
	if m.selected == nil {
		return
	}
	height := max(3, m.height-10)
	if m.returning {
		height = max(1, m.workspaceHeight()-8)
	}
	offset := m.viewport.YOffset()
	m.viewport.SetWidth(max(1, m.width-4))
	m.viewport.SetHeight(height)
	m.viewport.SoftWrap = true
	m.viewport.SetContent(m.reviewText())
	m.viewport.SetYOffset(offset)
}

func (m model) renderReview() string {
	r := m.selected
	m.refreshReviewViewport()
	header := fmt.Sprintf("REVIEW EXACT PLAN · %s\nPlan ID: %v\n", strings.ToUpper(safeTerminal(r.Direction)), r.Plan["id"])
	gates := "Consent gates unavailable · apply disabled"
	if r.GatesKnown {
		rows := []string{}
		for _, gate := range []struct {
			key, label    string
			required, ack bool
		}{{"u", "Unsafe changes", r.Unsafe, m.unsafeAck}, {"d", "Desktop integrations", r.Desktop, m.desktopAck}, {"b", "Backup replacement", r.Backup, m.backupAck}} {
			mark, required := " ", "not required"
			if gate.ack {
				mark = "x"
			}
			if gate.required {
				required = "required"
			}
			rows = append(rows, fmt.Sprintf("[%s] [%s] %s · %s", gate.key, mark, gate.label, required))
		}
		gates = strings.Join(rows, "\n")
	}
	if !m.returning && m.message != "" {
		gates += "\n" + safeTerminal(m.message)
	}
	return header + m.viewport.View() + "\n" + gates + "\nj/k scroll · p refresh exact plan · a continue · q cancel"
}

func (m model) renderMetadata() string {
	fields := []string{"destination", "os", "portability", "impact", "template", "git_keys", "removals", "tools", "warnings", "integrations"}
	currentField := fields[m.editField]
	currentVal := inputValue(m.editValue[currentField])

	var b strings.Builder
	fmt.Fprintf(&b, "EDIT %s METADATA · %s\n\n", strings.ToUpper(m.mode), safeTerminal(m.editPath))

	start, end := 0, len(fields)
	if m.returning {
		limit := max(3, min(6, m.workspaceHeight()-9))
		start = max(0, min(m.editField-2, len(fields)-limit))
		end = min(len(fields), start+limit)
	}
	for i := start; i < end; i++ {
		f := fields[i]
		cursor := "  "
		if i == m.editField {
			cursor = "> "
		}
		val := safeTerminal(fmt.Sprint(m.editValue[f]))
		if val == "" || val == "<nil>" {
			val = "(none)"
		}
		line := fmt.Sprintf("%s%-14s: %s", cursor, f, val)
		if m.returning {
			line = cellLine(line, m.width-4)
		}
		b.WriteString(line + "\n")
	}

	b.WriteString("\n")
	if guidance, ok := metadataGuidance[currentField]; ok {
		fmt.Fprintf(&b, "Guidance: %s\n", guidance)
	}
	if choices, ok := metadataChoices[currentField]; ok {
		var choiceStrs []string
		for _, c := range choices {
			label := c
			if label == "" {
				label = "(empty)"
			}
			if c == currentVal {
				choiceStrs = append(choiceStrs, fmt.Sprintf("[%s]", label))
			} else {
				choiceStrs = append(choiceStrs, label)
			}
		}
		fmt.Fprintf(&b, "Accepted choices: %s (Space/Left/Right to cycle)\n", strings.Join(choiceStrs, " · "))
	}

	if m.returning {
		return b.String()
	}
	b.WriteString("\nTab/Shift+Tab switch field · Space/Left/Right cycle choices · Type edit · Ctrl+U clear · Enter save · Esc cancel")
	if m.message != "" {
		b.WriteString("\n" + safeTerminal(m.message))
	}
	return b.String()
}

func Run(ctx context.Context, engine Engine, rounds bool, mode string) error {
	m := NewModel(engine, rounds, mode).(model)
	m.setupContext, m.setupCancel = context.WithCancel(ctx)
	defer m.setupCancel()
	program := tea.NewProgram(m, tea.WithContext(ctx))
	_, err := program.Run()
	return err
}

func RunInit(ctx context.Context, engine Engine) error {
	program := tea.NewProgram(NewInitModelContext(ctx, engine), tea.WithContext(ctx))
	_, err := program.Run()
	return err
}

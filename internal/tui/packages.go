package tui

import (
	"context"
	"fmt"
	"io"
	"sort"
	"strings"
	"unicode"

	tea "charm.land/bubbletea/v2"
)

// PackagesEngine is an optional independent seam: config-only engines do not
// gain package effects, and package approval cannot be sent through Apply.
type PackagesEngine interface {
	Packages(context.Context, string, map[string]any, any, string) (PackageResult, error)
}
type PackageRow struct {
	Status    string `json:"status"`
	Manager   string `json:"manager"`
	Kind      string `json:"kind"`
	Name      string `json:"name"`
	Desired   bool   `json:"desired"`
	Installed bool   `json:"installed"`
	Missing   bool   `json:"missing"`
}
type PackageSource struct {
	Text        string   `json:"text"`
	Digest      string   `json:"digest"`
	Exists      bool     `json:"exists"`
	Diagnostics []string `json:"diagnostics"`
}
type PackageResult struct {
	Notice   string                   `json:"notice"`
	Platform string                   `json:"platform"`
	Rows     []PackageRow             `json:"rows"`
	Sources  map[string]PackageSource `json:"sources"`
	Plan     map[string]any           `json:"plan"`
	Review   string                   `json:"review"`
}
type packagesMsg struct {
	generation uint64
	action     string
	result     PackageResult
	err        error
}

// Release Bubble Tea's raw terminal while managers ask for build review/sudo.
// The native adapter receives every terminal stream; config Apply is unrelated.
type packageInstall struct {
	engine  PackagesEngine
	ctx     context.Context
	plan    map[string]any
	streams TerminalStreams
}

func (p *packageInstall) SetStdin(r io.Reader)  { p.streams.In = r }
func (p *packageInstall) SetStdout(w io.Writer) { p.streams.Out = w }
func (p *packageInstall) SetStderr(w io.Writer) { p.streams.Err = w }
func (p *packageInstall) Run() error {
	e, ok := p.engine.(PackageTerminalEngine)
	if !ok {
		return fmt.Errorf("native terminal package installation unavailable; nothing installed")
	}
	return e.InstallPackages(p.ctx, p.plan, inputValue(p.plan["id"]), p.streams)
}

func (m *model) packageCommand(action string, request map[string]any) tea.Cmd {
	engine, ok := m.engine.(PackagesEngine)
	if !ok {
		m.message = "Package management is unavailable in this build."
		return nil
	}
	m.loading = true
	m.err = nil
	m.packageGeneration++
	generation := m.packageGeneration
	return func() tea.Msg {
		result, err := engine.Packages(m.setupCtx(), action, request, nil, "")
		return packagesMsg{generation, action, result, err}
	}
}
func (m model) updatePackages(key tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	if key.String() == "ctrl+c" {
		return m, tea.Quit
	}
	if m.packageStage != "edit" && m.packageStage != "import" {
		switch key.String() {
		case "q":
			if m.setupCancel != nil {
				m.setupCancel()
			}
			return m, tea.Quit
		case "P":
			m.packageView = false
			m.packageStage = ""
			m.packagePlan = nil
			m.loading = false
			m.err = nil
			m.packageGeneration++
			m.message = "Browsing is not authorization."
			return m, m.loadInventory
		case "esc":
			m.packageStage = ""
			m.packagePlan = nil
			m.loading = false
			m.packageGeneration++
			m.message = "Installation canceled; nothing installed."
			return m, nil
		}
	}
	if m.loading {
		return m, nil
	}
	if m.packageStage == "edit" || m.packageStage == "import" {
		if key.String() == "esc" {
			m.packageStage = ""
			m.message = "Edit canceled; private source unchanged."
			return m, nil
		}
		if key.String() == "ctrl+s" || tea.Key(key).Code == 19 {
			source := m.packages.Sources[m.packageManager]
			if m.packageStage == "import" {
				return m, m.packageCommand("import", map[string]any{"manager": m.packageManager, "digest": source.Digest, "supplied": strings.TrimSpace(m.templateInput.Value())})
			}
			text := m.templateInput.Value()
			if text == m.packageEditInitial {
				text = m.packageEditOriginal
			}
			return m, m.packageCommand("save", map[string]any{"manager": m.packageManager, "digest": source.Digest, "text": text})
		}
		var cmd tea.Cmd
		m.templateInput, cmd = m.templateInput.Update(key)
		return m, cmd
	}
	switch key.String() {
	case "enter":
		if m.packageStage != "confirm" {
			break
		}
		if m.confirmChoice == 0 {
			m.packageStage = ""
			m.packagePlan = nil
			m.message = "Installation canceled; nothing installed."
			break
		}
		engine, ok := m.engine.(PackagesEngine)
		if !ok || m.packagePlan == nil {
			break
		}
		m.loading = true
		m.packageGeneration++
		generation := m.packageGeneration
		return m, tea.Exec(&packageInstall{engine: engine, ctx: m.setupCtx(), plan: m.packagePlan}, func(err error) tea.Msg {
			return packagesMsg{generation: generation, action: "apply", err: err}
		})
	case "j", "down":
		if m.packageStage == "confirm" {
			m.confirmChoice = 1
		} else if m.packageStage == "review" {
			m.viewport.ScrollDown(1)
		} else {
			m.packageCursor = min(max(0, len(m.packages.Rows)-1), m.packageCursor+1)
		}
	case "k", "up":
		if m.packageStage == "confirm" {
			m.confirmChoice = 0
		} else if m.packageStage == "review" {
			m.viewport.ScrollUp(1)
		} else {
			m.packageCursor = max(0, m.packageCursor-1)
		}
	case "m":
		if m.packageStage != "" {
			break
		}
		managers := m.packageManagers()
		for i, manager := range managers {
			if manager == m.packageManager {
				m.packageManager = managers[(i+1)%len(managers)]
				break
			}
		}
	case "e":
		if m.packageStage != "" || m.packageManager == "" {
			break
		}
		source := m.packages.Sources[m.packageManager]
		m.templateInput.SetValue(source.Text)
		m.packageEditOriginal = source.Text
		m.packageEditInitial = m.templateInput.Value()
		m.templateInput.Focus()
		m.packageStage = "edit"
		m.message = ""
	case "L":
		if m.packageStage != "" || m.packageManager == "" {
			break
		}
		m.templateInput.SetValue("")
		m.templateInput.Focus()
		m.packageStage = "import"
		m.message = "Enter the supplied list's relative path inside the private checkout."
	case "r":
		m.packageStage = ""
		m.packagePlan = nil
		return m, m.packageCommand("status", nil)
	case "i":
		if m.packageStage != "" {
			break
		}
		m.packagePlan = nil
		m.message = "Preparing an exact missing-package review; no installation yet."
		return m, m.packageCommand("plan", nil)
	case "a":
		if m.packageStage == "review" && m.packagePlan != nil {
			m.packageStage = "confirm"
			m.confirmChoice = 0
		}
	}
	return m, nil
}
func (m model) packageManagers() []string {
	names := make([]string, 0, len(m.packages.Sources))
	for name := range m.packages.Sources {
		names = append(names, name)
	}
	sort.Strings(names)
	return names
}
func (m model) renderPackages() string {
	header := "DOTS-CLI · Dotfiles (P)  |  Packages\n"
	if m.packageStage == "import" {
		return header + "IMPORT PRIVATE PACKAGE SOURCE · " + m.packageManager + "\n" + packageEditorDisplay(m.templateInput.View()) + "\nCtrl+S import with backup · Esc cancel · no installation\n" + safeTerminal(m.message)
	}
	if m.packageStage == "edit" {
		return header + "PRIVATE PACKAGE SOURCE · " + m.packageManager + "\n" + packageEditorDisplay(m.templateInput.View()) + "\nCtrl+S save with backup · Esc cancel · saving never installs\n" + safeTerminal(m.message)
	}
	if m.packageStage == "review" {
		m.viewport.SetContent(safeTerminal(m.packageReview))
		m.viewport.SetHeight(max(3, m.height-6))
		return header + m.viewport.View() + "\nj/k scroll · a continue to consent · Esc cancel\n" + safeTerminal(m.message)
	}
	if m.packageStage == "confirm" {
		cancel, install := "> ", "  "
		if m.confirmChoice == 1 {
			cancel, install = "  ", "> "
		}
		return header + "INSTALL MISSING · FINAL CONSENT\nPlan ID: " + inputValue(m.packagePlan["id"]) + "\n\n" + cancel + "Cancel\n" + install + "Install exact reviewed missing packages\n\nManager dependencies and privilege/build prompts remain interactive.\nj/k choose · Enter confirm · Esc cancel"
	}
	var b strings.Builder
	b.WriteString(header + "Private desired lists · " + m.packages.Platform + " · source: " + m.packageManager + "\n\n")
	if m.loading {
		b.WriteString("Reading package state… no installation is authorized.\n")
	}
	if len(m.packages.Rows) == 0 && !m.loading {
		b.WriteString("No desired or installed packages to display. Press e to supply a private list.\n")
	}
	start := max(0, m.packageCursor-max(1, m.height-10)+1)
	for i, row := range m.packages.Rows[start:] {
		if i >= max(1, m.height-10) {
			break
		}
		cursor := "  "
		if start+i == m.packageCursor {
			cursor = "> "
		}
		state := "installed · not desired"
		if row.Desired {
			state = "desired · installed"
			if row.Missing {
				state = "desired · missing"
			}
		}
		if row.Status == "unavailable" {
			state = "desired · installed state unavailable"
		}
		kind := row.Manager
		if row.Manager == "brew" {
			kind = row.Kind
		}
		fmt.Fprintf(&b, "%s%s/%s  %s\n", cursor, kind, row.Name, state)
	}
	for _, manager := range m.packageManagers() {
		for _, d := range m.packages.Sources[manager].Diagnostics {
			fmt.Fprintf(&b, "%s: %s\n", manager, d)
		}
	}
	message := m.message
	if m.packages.Notice != "" {
		message = m.packages.Notice + "\n" + message
	}
	if m.err != nil {
		message = m.err.Error()
	}
	b.WriteString("\nEditing/saving never installs. Deselection never uninstalls.\n")
	if m.width < 100 {
		b.WriteString("i Install missing · e edit · L import · m source\nj/k move · r refresh · P Dotfiles · q quit\n")
	} else {
		b.WriteString("j/k move · m source · e edit · L import · r refresh · i Install missing · P Dotfiles · q quit\n")
	}
	b.WriteString(safeTerminal(message))
	return b.String()
}

func packageEditorDisplay(value string) string {
	return strings.Map(func(r rune) rune {
		if unicode.Is(unicode.Cf, r) {
			return -1
		}
		return r
	}, safeTerminal(value))
}

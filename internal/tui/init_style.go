package tui

import (
	"strings"

	"charm.land/bubbles/v2/progress"
	"charm.land/lipgloss/v2"
)

var (
	initTitle       = lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color(AppTheme.Title))
	initSubtitle    = lipgloss.NewStyle().Foreground(lipgloss.Color(AppTheme.Muted))
	initBody        = lipgloss.NewStyle().Foreground(lipgloss.Color(AppTheme.Text))
	initHint        = lipgloss.NewStyle().Foreground(lipgloss.Color(AppTheme.Muted))
	initActive      = lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color(AppTheme.Success))
	initDimStep     = lipgloss.NewStyle().Foreground(lipgloss.Color(AppTheme.Muted))
	initSelected    = lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color(AppTheme.Success))
	initLead        = lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color(AppTheme.Success))
	initSectionHead = lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color(AppTheme.Accent))
	initStatus      = lipgloss.NewStyle().Foreground(lipgloss.Color(AppTheme.Status))
)

// stepBar is a static bubbles progress bar: position in the wizard without
// any animation state or update wiring.
var stepBar = progress.New(
	progress.WithWidth(24),
	progress.WithoutPercentage(),
	progress.WithColors(lipgloss.Color(AppTheme.Accent)),
)

var wizardSteps = []string{"Welcome", "Source", "Machine", "Dependencies", "Tracker", "Done"}

// stepPosition renders the thin progress bar under the stepper.
func stepPosition(step string) string {
	idx := 0
	for i, s := range wizardSteps {
		if s == step {
			idx = i
		}
	}
	return stepBar.ViewAs(float64(idx) / float64(len(wizardSteps)))
}

func initSteps(current string) string {
	steps := wizardSteps
	var parts []string
	for _, s := range steps {
		if s == current {
			parts = append(parts, initActive.Render(s))
		} else {
			parts = append(parts, initDimStep.Render(s))
		}
	}
	return strings.Join(parts, initDimStep.Render(" → "))
}

func initChoiceList(choices []string, selected int) string {
	var b strings.Builder
	for i, c := range choices {
		if i == selected {
			// Single Render keeps "> <choice>" a contiguous substring.
			b.WriteString(initSelected.Render("> "+c) + "\n")
		} else {
			b.WriteString(initHint.Render("  "+c) + "\n")
		}
	}
	return strings.TrimSuffix(b.String(), "\n")
}

func initSection(title, body string) string {
	var b strings.Builder
	b.WriteString(initTitle.Render(title) + "\n")
	if body != "" {
		b.WriteString(initBody.Render(body) + "\n")
	}
	return b.String()
}

func initFooter(hints string, message string) string {
	var b strings.Builder
	b.WriteString("\n" + initHint.Render(hints))
	if strings.TrimSpace(message) != "" {
		b.WriteString("\n" + initStatus.Render(safeTerminal(message)))
	}
	return b.String()
}

// initShell shares the workspace palette and a bounded frame with the setup
// wizard. Very small terminals retain a compact layout for scrollable content.
func (m model) initShell(step, title, lead, body, choices, hints, message string) string {
	w, h := m.width, m.height
	if w < 1 {
		w = 80
	}
	if h < 1 {
		h = 24
	}
	if m.initCompact() {
		pad := m.shellPad(step, title, lead, body, choices, hints, message, w, h)
		return overlayStyle.Width(w).Render(m.shellContent(step, title, lead, body, choices, hints, message, pad))
	}
	pad := m.shellPad(step, title, lead, body, choices, hints, message, w-4, h-2)
	content := lipgloss.NewStyle().Width(max(1, w-4)).Render(m.shellContent(step, title, lead, body, choices, hints, message, pad))
	return panel("dot · Setup", fitHeight(content, max(1, h-2)), w, h, true)
}

func (m model) shellContent(step, title, lead, body, choices, hints, message string, pad int) string {
	var b strings.Builder
	b.WriteString(initSteps(step) + "\n" + stepPosition(step) + "\n\n")
	if title != "" {
		b.WriteString(initTitle.Render(title) + "\n")
	}
	if lead != "" {
		b.WriteString("\n" + initLead.Render(lead) + "\n")
	}
	if body != "" {
		b.WriteString("\n" + initBody.Render(body) + "\n")
	}
	if pad > 0 {
		b.WriteString(strings.Repeat("\n", pad))
	}
	if choices != "" {
		b.WriteString("\n" + choices + "\n")
	}
	b.WriteString(initFooter(hints, message))
	return b.String()
}

// shellPad measures the unpadded content and returns blank lines needed to pin
// choices and footer to the bottom. Zero when content already overflows.
func (m model) shellPad(step, title, lead, body, choices, hints, message string, w, h int) int {
	probe := overlayStyle.Width(w).Render(m.shellContent(step, title, lead, body, choices, hints, message, 0))
	if pad := h - (strings.Count(probe, "\n") + 1); pad > 0 {
		return pad
	}
	return 0
}

// initCompact drops the modal box on very small screens so viewport content
// and footer hints both stay reachable; styling and copy are unchanged.
func (m model) initCompact() bool {
	if m.width > 0 && m.width < 60 {
		return true
	}
	return m.height > 0 && m.height < 16
}

// Box chrome reserves vertical space; viewports shrink only when boxed.
func (m model) depHeight() int {
	if m.initCompact() {
		return max(3, m.height-9)
	}
	return max(3, m.height-13)
}

func (m model) trackerHeight() int {
	if m.initCompact() {
		return max(3, m.height-10)
	}
	return max(3, m.height-14)
}

func (m model) sourceConfirmHeight() int {
	if m.initCompact() {
		return max(3, m.height-7)
	}
	return max(3, m.height-14)
}

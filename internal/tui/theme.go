package tui

import (
	"os"
	"strings"
)

// Theme is the small curated UI palette for the whole TUI. Flows never read
// it; only render code does, so switching themes cannot authorize anything.
type Theme struct {
	Selection string
	Danger    string
	Name      string
	Bg        string // extra-dark app background painted behind overlays
	Mantle    string // slightly lighter panel wash
	Border    string // panel border
	Title     string // screen titles and headers
	Accent    string // active step and confirmations
	Text      string // primary body text
	Muted     string // hints, dividers, inactive steps
	Status    string // status and warning messages
	Success   string // selected choices and verified badges
}

// AvailableThemes includes the prototype palette under the existing "opencode"
// setting and three optional Catppuccin palettes. All share semantic roles.
func AvailableThemes() []Theme {
	return []Theme{
		{
			Name:      "opencode",
			Bg:        "#1A1B26",
			Mantle:    "#16161E",
			Selection: "#283457",
			Danger:    "#F7768E",
			Border:    "#3B4261",
			Title:     "#C0CAF5",
			Accent:    "#7AA2F7",
			Text:      "#C0CAF5",
			Muted:     "#929BB9",
			Status:    "#E0AF68",
			Success:   "#9ECE6A",
		},
		{
			Name:      "catppuccin-mocha",
			Selection: "#313244",
			Danger:    "#F38BA8",
			Bg:        "#11111B", // crust: the extra-dark default
			Mantle:    "#181825",
			Border:    "#CBA6F7", // mauve
			Title:     "#B4BEFE", // lavender
			Accent:    "#89B4FA", // blue
			Text:      "#CDD6F4",
			Muted:     "#7F849C", // overlay1
			Status:    "#F9E2AF", // yellow
			Success:   "#A6E3A1", // green
		},
		{
			Name:      "catppuccin-macchiato",
			Selection: "#363A4F",
			Danger:    "#ED8796",
			Bg:        "#181926",
			Mantle:    "#1E2030",
			Border:    "#C6A0F6",
			Title:     "#B8C0E0",
			Accent:    "#8AADF4",
			Text:      "#CAD3F5",
			Muted:     "#8087A2",
			Status:    "#EED49F",
			Success:   "#A6DA95",
		},
		{
			Name:      "catppuccin-frappe",
			Selection: "#414559",
			Danger:    "#E78284",
			Bg:        "#232634",
			Mantle:    "#292C3C",
			Border:    "#CA9EE6",
			Title:     "#BABBF1",
			Accent:    "#8CAAEE",
			Text:      "#C6D0F5",
			Muted:     "#838BA7",
			Status:    "#E5C890",
			Success:   "#A6D189",
		},
	}
}

// DefaultTheme matches the HTML prototype: dark surfaces, blue focus,
// amber status, green additions and red removals.
func DefaultTheme() Theme { return AvailableThemes()[0] }

// ResolveTheme honors DOT_THEME (flavor name or "mocha"/"macchiato"/"frappe"
// prefix) and falls back to the extra-dark default for anything else.
func ResolveTheme() Theme {
	want := strings.ToLower(strings.TrimSpace(os.Getenv("DOT_THEME")))
	if want == "" {
		return DefaultTheme()
	}
	for _, th := range AvailableThemes() {
		if th.Name == want || strings.HasSuffix(th.Name, "-"+want) {
			return th
		}
	}
	return DefaultTheme()
}

// AppTheme is resolved once at startup. Render code reads only this.
var AppTheme = ResolveTheme()

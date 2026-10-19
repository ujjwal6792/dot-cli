package acceptance

import (
	"strings"

	"github.com/charmbracelet/x/ansi"
)

// terminalScreen reconstructs the fixed-size acceptance PTY's visible text.
// Bubble Tea sends incremental cursor/erase operations: stripping ANSI from
// each chunk loses unchanged characters and cannot verify what a user sees.
// Styling and terminal capability requests do not affect these text assertions.
type terminalScreen struct {
	parser *ansi.Parser
	cells  [][]rune
	x, y   int
}

func newTerminalScreen(width, height int) *terminalScreen {
	s := &terminalScreen{parser: ansi.NewParser(), cells: make([][]rune, height)}
	for i := range s.cells {
		s.cells[i] = []rune(strings.Repeat(" ", width))
	}
	s.parser.SetHandler(ansi.Handler{Print: s.print, Execute: s.control, HandleCsi: s.csi})
	return s
}

func (s *terminalScreen) write(b []byte) {
	for _, c := range b {
		s.parser.Advance(c)
	}
}

func (s *terminalScreen) text() string {
	lines := make([]string, len(s.cells))
	for i, row := range s.cells {
		lines[i] = string(row)
	}
	return strings.Join(lines, "\n")
}

func (s *terminalScreen) print(r rune) {
	w := ansi.StringWidth(string(r))
	if w == 0 {
		return
	}
	if s.x+w > len(s.cells[0]) {
		s.x = 0
		s.linefeed()
	}
	s.cells[s.y][s.x] = r
	for i := 1; i < w; i++ {
		s.cells[s.y][s.x+i] = ' '
	}
	s.x += w
}

func (s *terminalScreen) linefeed() {
	if s.y < len(s.cells)-1 {
		s.y++
		return
	}
	copy(s.cells, s.cells[1:])
	s.cells[s.y] = []rune(strings.Repeat(" ", len(s.cells[0])))
}

func (s *terminalScreen) control(b byte) {
	switch b {
	case '\r':
		s.x = 0
	case '\n', '\v', '\f':
		s.linefeed()
	case '\b':
		s.x = max(0, s.x-1)
	case '\t':
		s.x = min(len(s.cells[0])-1, (s.x/8+1)*8)
	}
}

func (s *terminalScreen) erase(y, from, to int) {
	for x := max(0, from); x < min(len(s.cells[y]), to); x++ {
		s.cells[y][x] = ' '
	}
}

func (s *terminalScreen) csi(cmd ansi.Cmd, params ansi.Params) {
	if cmd.Prefix() != 0 || cmd.Intermediate() != 0 {
		return
	}
	p := func(i, fallback int) int { n, _, _ := params.Param(i, fallback); return n }
	n := max(1, p(0, 1))
	w, h := len(s.cells[0]), len(s.cells)
	switch cmd.Final() {
	case 'H', 'f':
		s.y, s.x = max(0, min(h-1, p(0, 1)-1)), max(0, min(w-1, p(1, 1)-1))
	case 'G', '`':
		s.x = max(0, min(w-1, n-1))
	case 'd':
		s.y = max(0, min(h-1, n-1))
	case 'A':
		s.y = max(0, s.y-n)
	case 'B':
		s.y = min(h-1, s.y+n)
	case 'C':
		s.x = min(w-1, s.x+n)
	case 'D':
		s.x = max(0, s.x-n)
	case 'E':
		s.x, s.y = 0, min(h-1, s.y+n)
	case 'F':
		s.x, s.y = 0, max(0, s.y-n)
	case 'X':
		s.erase(s.y, s.x, s.x+n)
	case 'K':
		switch p(0, 0) {
		case 0:
			s.erase(s.y, s.x, w)
		case 1:
			s.erase(s.y, 0, s.x+1)
		case 2:
			s.erase(s.y, 0, w)
		}
	case 'J':
		mode := p(0, 0)
		for y := range s.cells {
			if mode == 2 || mode == 0 && y > s.y || mode == 1 && y < s.y {
				s.erase(y, 0, w)
			} else if y == s.y {
				if mode == 0 {
					s.erase(y, s.x, w)
				} else if mode == 1 {
					s.erase(y, 0, s.x+1)
				}
			}
		}
	}
}

package acceptance

import (
	"strings"
	"testing"
)

func TestScreenPreservesUnchangedTextAcrossFragmentedRepaints(t *testing.T) {
	s := newTerminalScreen(100, 32)
	s.write([]byte("\x1b[?1049h\x1b[H\x1b[2JTRACK · ~\r\n[ ] keyboard-prefs"))
	for _, b := range []byte("\x1b[1;2H\x1b[38;5;111mracked Files · ~\x1b[m\x1b[2;2Hx") {
		s.write([]byte{b})
	}
	if text := s.text(); !strings.Contains(text, "Tracked Files · ~") || !strings.Contains(text, "[x] keyboard-prefs") {
		t.Fatalf("screen lost reused characters: %s", text)
	}
}

func TestScreenEraseMovementAndWrap(t *testing.T) {
	s := newTerminalScreen(5, 2)
	s.write([]byte("abcdef\r\x1b[K✓\x1b[1A\x1b[3G\x1b[2X"))
	if got := s.text(); got != "ab  e\n✓    " {
		t.Fatalf("unexpected screen: %q", got)
	}
}

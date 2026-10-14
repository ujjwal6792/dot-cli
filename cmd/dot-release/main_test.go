package main

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestInvalidBuildVersionCreatesNoOutput(t *testing.T) {
	out := filepath.Join(t.TempDir(), "output")
	if err := run([]string{"build", "--version", "1.2.3-..", "--out", out}); err == nil {
		t.Fatal("invalid version built")
	}
	if _, err := os.Stat(out); !os.IsNotExist(err) {
		t.Fatal("invalid version created output")
	}
}

func TestReleaseOperationsRejectMisplacedFlags(t *testing.T) {
	for _, args := range [][]string{{"verify", "--deps-dir", "/missing"}, {"build", "--archive", "/missing"}, {"extract", "--out", "/missing"}} {
		if err := run(args); err == nil || !strings.Contains(err.Error(), "unsupported") {
			t.Fatalf("%v: %v", args, err)
		}
	}
}

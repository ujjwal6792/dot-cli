package distribution_test

import (
	"encoding/json"
	"github.com/ujjwal6792/dots-cli/internal/distribution"
	"os"
	"path/filepath"
	"testing"
)

func TestUninstallRefusesForeignOwnershipAndPreservesBytes(t *testing.T) {
	root := t.TempDir()
	root, _ = filepath.EvalSymlinks(root)
	file := filepath.Join(root, "dot")
	os.WriteFile(file, []byte("Homebrew-owned fixture"), 0755)
	if err := distribution.Uninstall(root, true); err == nil {
		t.Fatal("unowned root accepted")
	}
	b, _ := os.ReadFile(file)
	if string(b) != "Homebrew-owned fixture" {
		t.Fatal("foreign installation changed")
	}
}

func TestForgedOrSymlinkedOwnerCannotRemoveForeignLauncher(t *testing.T) {
	for _, kind := range []string{"unknown-field", "wrong-root", "symlink"} {
		root := t.TempDir()
		root, _ = filepath.EvalSymlinks(root)
		bin := filepath.Join(root, "bin")
		os.Mkdir(bin, 0755)
		os.WriteFile(filepath.Join(bin, "dot"), []byte("foreign"), 0755)
		record := map[string]any{"schema": 1, "owner": "dot-native", "root": root, "uid": os.Geteuid()}
		if kind == "unknown-field" {
			record["manager"] = "brew"
		}
		if kind == "wrong-root" {
			record["root"] = "/foreign"
		}
		b, _ := json.Marshal(record)
		path := filepath.Join(root, "owner.json")
		if kind == "symlink" {
			outside := filepath.Join(t.TempDir(), "record")
			os.WriteFile(outside, b, 0600)
			os.Symlink(outside, path)
		} else {
			os.WriteFile(path, b, 0600)
		}
		if err := distribution.Uninstall(root, true); err == nil {
			t.Fatalf("accepted %s owner", kind)
		}
		got, _ := os.ReadFile(filepath.Join(bin, "dot"))
		if string(got) != "foreign" {
			t.Fatal("foreign launcher changed")
		}
	}
}

func TestWritableInstallationOwnershipFailsClosed(t *testing.T) {
	root := t.TempDir()
	root, _ = filepath.EvalSymlinks(root)
	record := map[string]any{"schema": 1, "owner": "dot-native", "root": root, "uid": os.Geteuid()}
	b, _ := json.Marshal(record)
	os.WriteFile(filepath.Join(root, "owner.json"), b, 0600)
	os.Chmod(root, 0777)
	if err := distribution.Uninstall(root, true); err == nil {
		t.Fatal("world-writable root authorized")
	}
}

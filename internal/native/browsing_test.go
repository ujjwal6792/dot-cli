package native_test

import (
	"context"
	"os"
	"path/filepath"
	"testing"
)

func TestBrowserInventoryLoadsOnlyExpandedHOMEFolders(t *testing.T) {
	e, c := fixture(t)
	dir := filepath.Join(c.Home, ".config", "editor")
	if err := os.MkdirAll(dir, 0700); err != nil {
		t.Fatal(err)
	}
	write(t, filepath.Join(dir, "prefs.conf"), "nested", 0600)
	write(t, filepath.Join(c.Home, ".config", "top.conf"), "direct", 0600)
	inv, err := e.BrowserInventory(context.Background(), "track")
	if err != nil {
		t.Fatal(err)
	}
	for _, item := range inv.Entries {
		if item.Path == ".config/editor" || item.Path == ".config/top.conf" {
			t.Fatalf("initial browser descended into %s", item.Path)
		}
	}
	children, err := e.BrowseDirectory(context.Background(), ".config")
	if err != nil {
		t.Fatal(err)
	}
	if len(children) != 2 {
		t.Fatalf("immediate children: %+v", children)
	}
	for _, item := range children {
		if item.Path == ".config/editor/prefs.conf" {
			t.Fatal("expansion recursively walked HOME")
		}
	}
	snapshot, err := e.Snapshot(context.Background(), ".config")
	if err != nil || len(snapshot.Paths) != 2 {
		t.Fatalf("exact recursive snapshot changed: %+v %v", snapshot, err)
	}
}

func TestBrowserExpansionEnforcesPrivateAndSymlinkBoundaries(t *testing.T) {
	e, c := fixture(t)
	if err := os.MkdirAll(filepath.Join(c.Home, "folder", "node_modules"), 0700); err != nil {
		t.Fatal(err)
	}
	if err := os.MkdirAll(filepath.Join(c.Home, "folder", ".ssh"), 0700); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(c.Source, filepath.Join(c.Home, "outside")); err != nil {
		t.Fatal(err)
	}
	for _, path := range []string{"../source", "outside", ".ssh", ".cache"} {
		if _, err := e.BrowseDirectory(context.Background(), path); err == nil {
			t.Fatalf("expanded unsafe directory %q", path)
		}
	}
	children, err := e.BrowseDirectory(context.Background(), "folder")
	if err != nil || len(children) != 0 {
		t.Fatalf("private/build directories leaked: %+v %v", children, err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	if _, err := e.BrowseDirectory(ctx, "folder"); err == nil {
		t.Fatal("ignored cancelled browse")
	}
}

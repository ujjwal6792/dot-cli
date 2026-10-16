package tui

import (
	"context"
	"errors"
	"strings"
	"testing"
)

type countingEngine struct {
	fixtureEngine
	inventories int
}

func (f *countingEngine) InventoryMode(ctx context.Context, mode string) (Inventory, error) {
	f.inventories++
	return f.fixtureEngine.InventoryMode(ctx, mode)
}

func TestEmptyRepositoryGuidesNextActionPerMode(t *testing.T) {
	m := boot(t, &fixtureEngine{inventory: Inventory{Entries: []Entry{}}}, false, "track")
	view := m.View().Content
	for _, want := range []string{"Empty repository", "press r to rescan"} {
		if !strings.Contains(view, want) {
			t.Fatalf("track empty missing %q:\n%s", want, view)
		}
	}
	m = boot(t, &fixtureEngine{inventory: Inventory{Entries: []Entry{}}}, false, "adopt")
	if view := m.View().Content; !strings.Contains(view, "Track files first") {
		t.Fatalf("adopt empty missing guidance:\n%s", view)
	}
}

type errInventoryEngine struct {
	fixtureEngine
	loadErr error
}

func (f *errInventoryEngine) InventoryMode(context.Context, string) (Inventory, error) {
	return Inventory{}, f.loadErr
}

func TestFailedLoadIsNotCalledEmpty(t *testing.T) {
	m := NewModel(&errInventoryEngine{loadErr: errors.New("boom")}, false, "track")
	next, _ := m.Update(m.Init()())
	view := next.View().Content
	if !strings.Contains(view, "Couldn't list files") {
		t.Fatalf("load failure not surfaced:\n%s", view)
	}
	if strings.Contains(view, "Empty repository") {
		t.Fatalf("load failure misreported as empty:\n%s", view)
	}
}

func TestRescanReloadsInventoryInPlace(t *testing.T) {
	e := &countingEngine{fixtureEngine: fixtureEngine{inventory: Inventory{Entries: []Entry{}}}}
	m := NewModel(e, false, "track")
	next, _ := m.Update(m.Init()())
	if e.inventories != 1 {
		t.Fatalf("boot loaded %d times", e.inventories)
	}
	next, cmd := next.Update(key("r"))
	if cmd == nil {
		t.Fatal("r did not trigger a rescan")
	}
	next, _ = next.Update(cmd())
	if e.inventories != 2 {
		t.Fatalf("rescan loaded %d times", e.inventories)
	}
	_ = next
}

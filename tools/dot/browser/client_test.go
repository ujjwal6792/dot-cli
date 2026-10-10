package browser

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestClientSendsExplicitEngineAndSourceForInventory(t *testing.T) {
	fixture := t.TempDir()
	engine := filepath.Join(fixture, "public", "tools", "dot")
	source := filepath.Join(fixture, "private")
	home := filepath.Join(fixture, "home")
	for _, path := range []string{engine, source, home} {
		if err := os.MkdirAll(path, 0o755); err != nil {
			t.Fatal(err)
		}
	}
	program := filepath.Join(fixture, "engine")
	log := filepath.Join(fixture, "request.json")
	script := "import json, os, sys\nrequest=json.load(sys.stdin)\nopen(os.environ['REQUEST_LOG'],'w').write(json.dumps(request))\nprint(json.dumps({'version':1,'ok':True,'result':{'entries':[{'path':'.config','kind':'directory','status':'eligible'}]}}))\n"
	if err := os.WriteFile(program, []byte(script), 0o755); err != nil {
		t.Fatal(err)
	}
	c := Client{Executable: program, Engine: engine, Source: source, Home: home}
	t.Setenv("REQUEST_LOG", log)
	got, err := c.InventoryMode(context.Background(), "adopt")
	if err != nil {
		t.Fatal(err)
	}
	if len(got.Entries) != 1 || got.Entries[0].Path != ".config" {
		t.Fatalf("unexpected inventory: %#v", got)
	}
	request, err := os.ReadFile(log)
	if err != nil {
		t.Fatal(err)
	}
	for _, expected := range []string{`"command": "inventory"`, `"engine": "` + engine + `"`, `"source": "` + source + `"`, `"home": "` + home + `"`, `"mode": "adopt"`} {
		if !strings.Contains(string(request), expected) {
			t.Errorf("request %s missing %s", request, expected)
		}
	}
}

func TestClientRejectsUnsupportedBridgeResponseVersion(t *testing.T) {
	fixture := t.TempDir()
	program := filepath.Join(fixture, "engine")
	script := "import json; json.load(__import__('sys').stdin); print(json.dumps({'version':2,'ok':True,'result':{'entries':[{'path':'unexpected','kind':'file','status':'eligible'}]}}))\n"
	if err := os.WriteFile(program, []byte(script), 0o755); err != nil {
		t.Fatal(err)
	}
	c := Client{Executable: program, Engine: filepath.Join(fixture, "engine-root"), Source: filepath.Join(fixture, "source"), Home: filepath.Join(fixture, "home")}
	_, err := c.InventoryMode(context.Background(), "track")
	if err == nil || !strings.Contains(err.Error(), "unsupported engine response version 2") {
		t.Fatalf("expected protocol-version rejection, got %v", err)
	}
}

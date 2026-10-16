package acceptance

import (
	"strings"
	"testing"

	"github.com/ujjwal6792/dots-cli/internal/native"
)

func TestStrictDecodeFailsClosed(t *testing.T) {
	var m native.Manifest
	if err := native.Decode([]byte(`{"version":1,"entries":{},"profiles":{},"unknown":true}`), &m); err == nil {
		t.Fatal("unknown manifest field accepted")
	}
	var a native.Adoption
	if err := native.Decode([]byte(`{"version":3,"machine":{"id":"macos_one","os":"darwin"},"entries":{},"profile":"base","unknown":true}`), &a); err == nil {
		t.Fatal("unknown adoption field accepted")
	}
	var r native.Request
	if err := native.Decode([]byte(`{"version":1,"mode":"adopt","profile":"alpha","entries":["all_a"],"unknown":true}`), &r); err == nil {
		t.Fatal("unknown request field accepted")
	}
	var p native.Plan
	good := `{"version":1,"id":"x","request":{"version":1,"mode":"adopt","profile":"alpha","entries":[]},"machine":{"id":"macos_one","os":"darwin"},"selected":{},"actions":[],"gates":{"unsafe":false,"desktop":false,"backup":false},"fingerprint":"","direction":""}`
	if err := native.Decode([]byte(good), &p); err != nil {
		t.Fatalf("valid plan rejected: %v", err)
	}
	badPlan := `{"version":1,"id":"x","request":{"version":1,"mode":"adopt","profile":"alpha","entries":[]},"machine":{"id":"macos_one","os":"darwin"},"selected":{},"actions":[],"gates":{"unsafe":false,"desktop":false,"backup":false},"fingerprint":"","direction":"","unknown":true}`
	if err := native.Decode([]byte(badPlan), &p); err == nil {
		t.Fatal("unknown plan field accepted")
	}
	badGates := `{"version":1,"id":"x","request":{"version":1,"mode":"adopt","profile":"alpha","entries":[]},"machine":{"id":"macos_one","os":"darwin"},"selected":{},"actions":[],"gates":{"unsafe":false,"desktop":false,"backup":false,"unknown":true},"fingerprint":"","direction":""}`
	if err := native.Decode([]byte(badGates), &p); err == nil {
		t.Fatal("unknown gate field accepted")
	}
}

func TestManagerFixtureNeverInstalls(t *testing.T) {
	for _, denied := range []string{"sudo", "apt-get", "pacman", "yum", "dnf", "brew install", "exec.Command", "os/exec", "syscall.Exec"} {
		if strings.Contains(managerFixture, denied) {
			t.Fatalf("manager fixture may install: %q", denied)
		}
	}
	if !strings.Contains(managerFixture, "manager-call.json") {
		t.Fatal("manager fixture must record invocation")
	}
}

func TestOpenLabelSanitized(t *testing.T) {
	for _, s := range []string{sanitizeOpenLabel("Welcome-"), sanitizeOpenLabel("What next?-init"), sanitizeOpenLabel("  ")} {
		if s == "" || strings.ContainsAny(s, " /") {
			t.Fatalf("unsanitized open label %q", s)
		}
	}
	if sanitizeOpenLabel("Welcome") != "welcome" {
		t.Fatalf("unexpected label %q", sanitizeOpenLabel("Welcome"))
	}
}

func TestMustMarshalRoundTrip(t *testing.T) {
	b := mustMarshal(map[string]any{"a": 1})
	var v map[string]any
	if err := native.Decode(b, &v); err != nil {
		t.Fatalf("mustMarshal produced invalid JSON: %v", err)
	}
}

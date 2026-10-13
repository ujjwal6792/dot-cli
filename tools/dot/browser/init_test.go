package browser

import (
	"strings"
	"testing"
)

func TestInitWelcomeAndDoneRenderWithoutEngineCalls(t *testing.T) {
	e := &fixtureEngine{}
	m := NewInitModel(e)
	if cmd := m.Init(); cmd != nil {
		t.Fatal("welcome must render without inventory before choice")
	}
	if view := m.View().Content; !strings.Contains(view, "Welcome") {
		t.Fatalf("welcome page missing: %s", view)
	}
	m = send(m, "enter")
	if view := m.View().Content; !strings.Contains(view, "Done") {
		t.Fatalf("done page missing after welcome: %s", view)
	}
	if e.plans != 0 || e.previews != 0 || e.applies != 0 {
		t.Fatal("welcome/done invoked engine plan/preview/apply")
	}
}

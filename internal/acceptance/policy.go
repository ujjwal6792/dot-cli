package acceptance

import (
	"bytes"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"sync"

	"github.com/ujjwal6792/dots-cli/internal/native"
)

func (h harness) scope(home string) (native.Adoption, error) {
	var a native.Adoption
	b, err := h.invoke(home, "", "scope", "show", "--format", "json")
	if err == nil {
		err = native.Decode(b, &a)
	}
	return a, err
}

func mustMarshal(v any) []byte { b, _ := json.Marshal(v); return b }

func (h harness) policyCases(one, two string) error {
	source := filepath.Join(two, ".local/share/dot-config")
	manifestPath := filepath.Join(source, "manifest.json")
	var m native.Manifest
	b, err := os.ReadFile(manifestPath)
	if err != nil {
		return err
	}
	m = native.Manifest{}
	if err = native.Decode(b, &m); err != nil {
		return err
	}
	beforeOne, err := h.scope(one)
	if err != nil {
		return err
	}
	// All content is enrolled through real packaged Track, not a fixture success flag.
	for _, item := range []struct {
		id, profile string
		assignment  native.Assignment
	}{
		{"all_a", "alpha", native.Assignment{Include: []string{"all"}, Exclude: []string{}}},
		{"all_b", "beta", native.Assignment{Include: []string{"all"}, Exclude: []string{}}},
		{"only_one", "alpha", native.Assignment{Include: []string{"macos_one"}, Exclude: []string{}}},
		{"excluded_two", "beta", native.Assignment{Include: []string{"all"}, Exclude: []string{"macos_two"}}},
	} {
		if err = write(filepath.Join(two, item.id), []byte("bytes \x00\xff "+item.id+"\r\n"), 0751); err != nil {
			return err
		}
		p, e := h.plan(two, native.Request{Version: 1, Mode: "track", Profile: item.profile, Selections: []native.Selection{{Path: item.id, Entry: native.Entry{OS: []string{runtime.GOOS}, Portability: "managed-shared", Impact: "Safe", Machines: &item.assignment}}}})
		if e != nil {
			return e
		}
		if e = h.apply(two, p); e != nil {
			return e
		}
	}
	for _, id := range []string{"only_one", "excluded_two"} {
		if _, err = h.plan(two, native.Request{Version: 1, Mode: "adopt", Profile: "alpha", Entries: []string{id}}); err == nil {
			return fmt.Errorf("machine assignment allowed %s", id)
		}
	}
	p, err := h.plan(two, native.Request{Version: 1, Mode: "adopt", Profile: "union", Profiles: []string{"alpha", "beta"}, Entries: []string{"all_a"}, Metadata: map[string]map[string]any{"all_a": {"notices": []string{"machine-local notice"}}}})
	if err != nil {
		return err
	}
	if err = h.apply(two, p); err != nil {
		return err
	}
	a, err := h.scope(two)
	if err != nil {
		return err
	}
	if len(a.Profiles) != 2 || len(a.Entries) != 1 || len(a.Deselected) != 1 || a.Deselected[0] != "all_b" || len(a.Customs) != 1 {
		return fmt.Errorf("profile/local intent lost: %+v", a)
	}
	p, err = h.plan(two, native.Request{Version: 1, Mode: "adopt", Profile: "alpha", Entries: []string{"all_a"}})
	if err != nil {
		return err
	}
	if err = h.apply(two, p); err != nil {
		return err
	}
	p, err = h.plan(two, native.Request{Version: 1, Mode: "adopt", Profile: "union", Profiles: []string{"alpha", "beta"}})
	if err != nil || len(p.Selected) != 1 {
		return fmt.Errorf("profile switch forgot deselection: %+v %v", p.Selected, err)
	}
	afterOne, err := h.scope(one)
	if err != nil || marshal(beforeOne) != marshal(afterOne) {
		return fmt.Errorf("machine two changed machine one adoption")
	}
	b, err = os.ReadFile(manifestPath)
	if err != nil {
		return err
	}
	if bytes.Contains(b, []byte("machine-local notice")) {
		return fmt.Errorf("local customs leaked to shared manifest")
	}
	// Future profile members must not join existing exact Update approval.
	if err = write(filepath.Join(two, "future"), []byte("future"), 0640); err != nil {
		return err
	}
	p, err = h.plan(two, native.Request{Version: 1, Mode: "track", Profile: "alpha", Selections: []native.Selection{{Path: "future", Entry: native.Entry{OS: []string{runtime.GOOS}, Portability: "managed-shared", Impact: "Safe"}}}})
	if err != nil {
		return err
	}
	if err = h.apply(two, p); err != nil {
		return err
	}
	p, err = h.plan(two, native.Request{Version: 1, Mode: "update", Profile: "alpha"})
	if err != nil || len(p.Selected) != 1 || p.Selected["all_a"].Destination != "all_a" {
		return fmt.Errorf("Update enrolled future profile members: %v", err)
	}
	// Unsafe, desktop effects and an existing backup slot require independent gates.
	b, err = os.ReadFile(manifestPath)
	if err != nil {
		return err
	}
	m = native.Manifest{}
	if err = native.Decode(b, &m); err != nil {
		return err
	}
	m.Entries["gated"] = native.Entry{Source: "home/gated", Destination: "gated", OS: []string{runtime.GOOS}, Portability: "managed-shared", Impact: "Unsafe", Warnings: []string{"desktop session changes"}, Integrations: []string{"desktop"}}
	m.Profiles["gated"] = []string{"gated"}
	if err = write(filepath.Join(source, "home/gated"), []byte("reviewed\x00\xff\n"), 0751); err != nil {
		return err
	}
	if err = write(manifestPath, []byte(marshal(m)), 0600); err != nil {
		return err
	}
	if err = write(filepath.Join(two, "gated"), []byte("human original"), 0640); err != nil {
		return err
	}
	if err = write(filepath.Join(two, "gated.dot-backup"), []byte("older backup"), 0600); err != nil {
		return err
	}
	p, err = h.plan(two, native.Request{Version: 1, Mode: "adopt", Profile: "gated", Entries: []string{"gated"}})
	if err != nil {
		return err
	}
	if !p.Gates.Unsafe || !p.Gates.Desktop || !p.Gates.Backup {
		return fmt.Errorf("missing independent gates: %+v", p.Gates)
	}
	for _, flags := range [][]string{{"--ack-desktop", "--ack-backup"}, {"--ack-unsafe", "--ack-backup"}, {"--ack-unsafe", "--ack-desktop"}} {
		args := append([]string{"intent", "apply", "--plan", "-", "--approve", p.ID}, flags...)
		if _, err = h.invoke(two, marshal(p), args...); err == nil {
			return fmt.Errorf("gate omission allowed apply: %v", flags)
		}
		live, _ := os.ReadFile(filepath.Join(two, "gated"))
		backup, _ := os.ReadFile(filepath.Join(two, "gated.dot-backup"))
		if string(live) != "human original" || string(backup) != "older backup" {
			return fmt.Errorf("rejected gate mutated bytes")
		}
	}
	if err = h.apply(two, p); err != nil {
		return err
	}
	live, _ := os.ReadFile(filepath.Join(two, "gated"))
	backup, _ := os.ReadFile(filepath.Join(two, "gated.dot-backup"))
	st, _ := os.Stat(filepath.Join(two, "gated"))
	if string(live) != "reviewed\x00\xff\n" || string(backup) != "human original" || st.Mode().Perm() != 0751 {
		return fmt.Errorf("gated apply bytes/modes/backups mismatch")
	}
	// Simultaneous processes cannot consume the same changing approval twice.
	if err = write(filepath.Join(two, "gated"), []byte("concurrent original"), 0640); err != nil {
		return err
	}
	p, err = h.plan(two, native.Request{Version: 1, Mode: "update", Profile: "gated"})
	if err != nil {
		return err
	}
	var wg sync.WaitGroup
	results := make(chan error, 2)
	for i := 0; i < 2; i++ {
		wg.Go(func() {
			_, e := h.invoke(two, marshal(p), "intent", "apply", "--plan", "-", "--approve", p.ID, "--ack-unsafe", "--ack-desktop", "--ack-backup")
			results <- e
		})
	}
	wg.Wait()
	close(results)
	successes := 0
	for e := range results {
		if e == nil {
			successes++
		}
	}
	if successes != 1 {
		return fmt.Errorf("concurrent approvals succeeded %d times", successes)
	}
	// Durable partial-action fixture proves real recovery, including conflict refusal.
	if err = write(filepath.Join(two, "gated"), []byte("interrupted original"), 0640); err != nil {
		return err
	}
	p, err = h.plan(two, native.Request{Version: 1, Mode: "update", Profile: "gated"})
	if err != nil {
		return err
	}
	journal := native.Journal{Plan: p, Actions: p.Actions}
	var interrupted native.Action
	for _, action := range p.Actions {
		if action.Root == "home" && action.Path == "gated" {
			interrupted = action
		}
	}
	if interrupted.Path == "" {
		return fmt.Errorf("missing interrupted home action")
	}
	// Simulate durable state after the first exact write, before adoption promotion.
	if err = write(filepath.Join(two, "gated"), interrupted.After.Data, interrupted.After.Mode); err != nil {
		return err
	}
	if err = os.Chmod(filepath.Join(two, "gated"), interrupted.After.Mode); err != nil {
		return err
	}
	if err = write(filepath.Join(two, interrupted.BackupPath), interrupted.Before.Data, interrupted.Before.Mode); err != nil {
		return err
	}
	if err = os.Chmod(filepath.Join(two, interrupted.BackupPath), interrupted.Before.Mode); err != nil {
		return err
	}
	// The interrupted plan is now stale because its first action already ran.
	// Use an otherwise current plan to prove recovery itself blocks Apply.
	blockedPlan, err := h.plan(two, native.Request{Version: 1, Mode: "update", Profile: "gated"})
	if err != nil {
		return err
	}
	if err = write(filepath.Join(two, ".local/state/dot/recovery.json"), []byte(marshal(journal)), 0600); err != nil {
		return err
	}
	if _, err = h.invoke(two, marshal(blockedPlan), "intent", "apply", "--plan", "-", "--approve", blockedPlan.ID, "--ack-unsafe", "--ack-desktop", "--ack-backup"); err == nil {
		return fmt.Errorf("pending recovery allowed apply")
	} else if !strings.Contains(strings.ToLower(err.Error()), "recover") && !strings.Contains(strings.ToLower(err.Error()), "interrupted") {
		return fmt.Errorf("pending apply refused without recovery signal: %v", err)
	}
	// Pending recovery must also block remote operations, not just apply.
	if _, err = h.invoke(two, "", "pull", "--yes"); err == nil {
		return fmt.Errorf("pending recovery allowed pull")
	} else if !strings.Contains(strings.ToLower(err.Error()), "recover") && !strings.Contains(strings.ToLower(err.Error()), "interrupted") {
		return fmt.Errorf("pending pull refused without recovery signal: %v", err)
	}
	if _, err = h.invoke(two, "", "push", "--yes"); err == nil {
		return fmt.Errorf("pending recovery allowed push")
	} else if !strings.Contains(strings.ToLower(err.Error()), "recover") && !strings.Contains(strings.ToLower(err.Error()), "interrupted") {
		return fmt.Errorf("pending push refused without recovery signal: %v", err)
	}
	live, _ = os.ReadFile(filepath.Join(two, "gated"))
	if !bytes.Equal(live, interrupted.After.Data) {
		return fmt.Errorf("blocked remote changed target during pending recovery")
	}
	if err = write(filepath.Join(two, "gated"), []byte("post-interruption human edit"), 0640); err != nil {
		return err
	}
	if _, err = h.invoke(two, "", "recover", "--approve", p.ID); err == nil {
		return fmt.Errorf("recovery overwrote human edit")
	}
	if err = write(filepath.Join(two, "gated"), interrupted.After.Data, interrupted.After.Mode); err != nil {
		return err
	}
	if err = os.Chmod(filepath.Join(two, "gated"), interrupted.After.Mode); err != nil {
		return err
	}
	if _, err = h.invoke(two, "", "recover", "--approve", p.ID); err != nil {
		return err
	}
	live, _ = os.ReadFile(filepath.Join(two, "gated"))
	if string(live) != "interrupted original" {
		return fmt.Errorf("recovery did not preserve before bytes")
	}
	backup, _ = os.ReadFile(filepath.Join(two, interrupted.BackupPath))
	st, _ = os.Stat(filepath.Join(two, "gated"))
	if !bytes.Equal(backup, interrupted.BackupBefore.Data) || st.Mode().Perm() != interrupted.Before.Mode.Perm() {
		return fmt.Errorf("interrupted recovery did not restore backup/mode")
	}
	// Malformed shared/private state and unsafe paths fail closed without widening scope.
	goodManifest, _ := os.ReadFile(manifestPath)
	for _, bad := range []string{`{"version":1,"entries":{},"profiles":{},"unknown":true}`, `{"version":1,"entries":{"bad":{"source":"../escape","destination":"bad","os":["darwin"],"portability":"managed-shared","impact":"Safe"}},"profiles":{"base":["bad"]}}`} {
		if err = write(manifestPath, []byte(bad), 0600); err != nil {
			return err
		}
		if _, err = h.invoke(two, "", "contract", "check"); err == nil {
			return fmt.Errorf("malformed/unsafe manifest accepted")
		}
	}
	if err = write(manifestPath, goodManifest, 0600); err != nil {
		return err
	}
	state := filepath.Join(two, ".local/state/dot/adoption.json")
	goodState, _ := os.ReadFile(state)
	if err = write(state, []byte(`{"version":3,"machine":{"id":"wrong","os":"darwin"},"entries":{},"profile":"gated","unknown":true}`), 0600); err != nil {
		return err
	}
	if _, err = h.invoke(two, "", "status"); err == nil {
		return fmt.Errorf("malformed adoption accepted")
	}
	if err = write(state, goodState, 0600); err != nil {
		return err
	}
	for _, assignment := range []string{`[]`, `{"include":[],"exclude":[]}`, `{"include":["*"],"exclude":[]}`, `{"include":["all"],"exclude":[],"unknown":true}`} {
		request := `{"version":1,"mode":"track","profile":"alpha","entries":[],"selections":[{"path":"all_a","os":["darwin"],"portability":"managed-shared","impact":"Safe","machines":` + assignment + `}]}`
		if _, err = h.invoke(two, request, "intent", "plan", "--request", "-"); err == nil {
			return fmt.Errorf("malformed assignment accepted: %s", assignment)
		}
	}
	if err = os.Symlink(filepath.Join(source, "home/gated"), filepath.Join(two, "unsafe-link")); err != nil {
		return err
	}
	if _, err = h.plan(two, native.Request{Version: 1, Mode: "track", Profile: "alpha", Selections: []native.Selection{{Path: "unsafe-link", Entry: native.Entry{OS: []string{runtime.GOOS}, Portability: "managed-shared", Impact: "Safe"}}}}); err == nil {
		return fmt.Errorf("symlink enrollment accepted")
	}
	_ = os.Remove(filepath.Join(two, "unsafe-link"))
	// Conflicting destinations fail closed without widening scope.
	m = native.Manifest{}
	if err = native.Decode(goodManifest, &m); err != nil {
		return err
	}
	colliding := m
	if len(colliding.Entries) == 0 {
		return fmt.Errorf("no entries for collision check")
	}
	var firstID string
	var first native.Entry
	for id, x := range colliding.Entries {
		firstID, first = id, x
		break
	}
	colliding.Entries["collide-destination"] = first
	colliding.Profiles["collide"] = []string{"collide-destination", firstID}
	if err = write(manifestPath, []byte(marshal(colliding)), 0600); err != nil {
		return err
	}
	if _, err = h.invoke(two, "", "contract", "check"); err == nil {
		return fmt.Errorf("destination collision accepted")
	}
	if err = write(manifestPath, goodManifest, 0600); err != nil {
		return err
	}
	// Unknown request/plan/gate fields fail closed (strict Decode).
	if _, err = h.invoke(two, `{"version":1,"mode":"adopt","profile":"alpha","entries":["all_a"],"unknown":true}`, "intent", "plan", "--request", "-"); err == nil {
		return fmt.Errorf("unknown request field accepted")
	}
	known, err := h.plan(two, native.Request{Version: 1, Mode: "adopt", Profile: "alpha", Entries: []string{"all_a"}})
	if err != nil {
		return err
	}
	var planMap map[string]any
	rawPlan, _ := json.Marshal(known)
	if err = json.Unmarshal(rawPlan, &planMap); err != nil {
		return err
	}
	planMap["unknown"] = true
	if _, err = h.invoke(two, string(mustMarshal(planMap)), "intent", "preview", "--plan", "-"); err == nil {
		return fmt.Errorf("unknown plan field accepted")
	}
	planMap = map[string]any{}
	if err = json.Unmarshal(rawPlan, &planMap); err != nil {
		return err
	}
	planMap["gates"] = map[string]any{"unsafe": false, "desktop": false, "backup": false, "unknown": true}
	if _, err = h.invoke(two, string(mustMarshal(planMap)), "intent", "preview", "--plan", "-"); err == nil {
		return fmt.Errorf("unknown gate field accepted")
	}
	// Unsupported helpers stay fail closed; never execute retired template runners.
	m = native.Manifest{}
	if err = native.Decode(goodManifest, &m); err != nil {
		return err
	}
	m.Entries["helper"] = native.Entry{Source: "home/helper", Destination: "helper", OS: []string{runtime.GOOS}, Portability: "managed-templated", Impact: "Safe"}
	m.Profiles["helper"] = []string{"helper"}
	if err = write(filepath.Join(source, "home/helper"), []byte(`{{ output "retired-runtime" }}`), 0644); err != nil {
		return err
	}
	if err = write(manifestPath, []byte(marshal(m)), 0600); err != nil {
		return err
	}
	if _, err = h.plan(two, native.Request{Version: 1, Mode: "adopt", Profile: "helper", Entries: []string{"helper"}}); err == nil {
		return fmt.Errorf("unsupported helper template rendered")
	}
	if _, err = os.Stat(filepath.Join(two, "helper")); !os.IsNotExist(err) {
		return fmt.Errorf("helper rejection wrote target")
	}
	if err = write(manifestPath, goodManifest, 0600); err != nil {
		return err
	}
	// Dirty checkout must refuse pull with a usable remote, before any offline test.
	if _, err = h.invoke(two, "", "pull", "--yes"); err == nil {
		return fmt.Errorf("dirty pull succeeded with usable remote")
	} else if !strings.Contains(strings.ToLower(err.Error()), "dirty") && !strings.Contains(strings.ToLower(err.Error()), "untracked") {
		return fmt.Errorf("dirty pull refused without dirty signal: %v", err)
	}
	live, _ = os.ReadFile(filepath.Join(two, "gated"))
	if string(live) != "interrupted original" {
		return fmt.Errorf("refused dirty pull changed target")
	}
	// Inspection cannot need remote access: make the transport unusable, then inspect.
	if err = h.gitRun(two, "-C", source, "remote", "set-url", "origin", "https://offline.invalid/config.git"); err != nil {
		return err
	}
	for _, args := range [][]string{{"init", "--check"}, {"contract", "check"}, {"status"}, {"diff"}, {"scope", "show"}} {
		if _, err = h.invoke(two, "", args...); err != nil {
			return err
		}
	}
	if _, err = h.invoke(two, "", "pull", "--yes"); err == nil {
		return fmt.Errorf("dirty pull succeeded")
	}
	live, _ = os.ReadFile(filepath.Join(two, "gated"))
	if string(live) != "interrupted original" {
		return fmt.Errorf("inspection/pull changed target")
	}
	// Secret-bearing URL errors must not echo secret values or create a source.
	secretHome := filepath.Join(h.dir, "home-secret")
	if err = os.Mkdir(secretHome, 0700); err != nil {
		return err
	}
	for _, url := range []string{"https://user:supersecret@host.invalid/config", "https://host.invalid/config?supersecret", "ssh://git:supersecret@host.invalid/config"} {
		command := exec.CommandContext(h.ctx, h.dot, "init", "--action", "clone", "--url", url)
		command.Dir = h.dir
		command.Env = h.env(secretHome)
		output, e := command.CombinedOutput()
		err = e
		if err == nil {
			return fmt.Errorf("credential URL accepted")
		}
		if strings.Contains(string(output), "supersecret") {
			return fmt.Errorf("URL rejection exposed credentials")
		}
	}
	if _, err = os.Stat(filepath.Join(secretHome, ".local")); !os.IsNotExist(err) {
		return fmt.Errorf("rejected URL wrote source/state")
	}
	// Stop tracking removes shared enrollment only, preserving content and the
	// machine's existing exact approval for explicit later resolution.
	beforeScope, err := h.scope(two)
	if err != nil {
		return err
	}
	beforeTarget, err := os.ReadFile(filepath.Join(two, "all_a"))
	if err != nil {
		return err
	}
	beforeSource, err := os.ReadFile(filepath.Join(source, "home/all_a"))
	if err != nil {
		return err
	}
	beforeMode, err := os.Stat(filepath.Join(two, "all_a"))
	if err != nil {
		return err
	}
	p, err = h.plan(two, native.Request{Version: 1, Mode: "untrack", Profile: "alpha", Entries: []string{"all_a"}})
	if err != nil {
		return err
	}
	if err = h.apply(two, p); err != nil {
		return err
	}
	afterScope, err := h.scope(two)
	if err != nil || marshal(beforeScope) != marshal(afterScope) {
		return fmt.Errorf("untrack changed local approval: %v", err)
	}
	afterTarget, err := os.ReadFile(filepath.Join(two, "all_a"))
	if err != nil || !bytes.Equal(beforeTarget, afterTarget) {
		return fmt.Errorf("untrack changed target bytes: %v", err)
	}
	afterSource, err := os.ReadFile(filepath.Join(source, "home/all_a"))
	if err != nil || !bytes.Equal(beforeSource, afterSource) {
		return fmt.Errorf("untrack discarded source bytes: %v", err)
	}
	afterMode, err := os.Stat(filepath.Join(two, "all_a"))
	if err != nil || beforeMode.Mode().Perm() != afterMode.Mode().Perm() {
		return fmt.Errorf("untrack changed target mode: %v", err)
	}
	b, err = os.ReadFile(manifestPath)
	if err != nil {
		return err
	}
	m = native.Manifest{}
	if err = native.Decode(b, &m); err != nil {
		return err
	}
	if _, exists := m.Entries["all_a"]; exists {
		return fmt.Errorf("untrack retained shared enrollment")
	}
	return nil
}

package tui

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"sort"
	"strings"

	"github.com/ujjwal6792/dots-cli/internal/native"
)

// NativeEngine is a thin view adapter, not another policy implementation. Setup
// can embed it and implement its own optional Source/Machine/Dependency seams.
type NativeEngine struct{ Policy *native.Engine }

func NewNativeEngine(e *native.Engine) *NativeEngine { return &NativeEngine{Policy: e} }

var _ Engine = (*NativeEngine)(nil)

func convert(in, out any) error {
	b, err := json.Marshal(in)
	if err != nil {
		return err
	}
	return json.Unmarshal(b, out)
}
func strictConvert(in, out any) error {
	b, err := json.Marshal(in)
	if err != nil {
		return err
	}
	return native.Decode(b, out)
}
func (e *NativeEngine) InventoryMode(ctx context.Context, mode string) (Inventory, error) {
	inv, err := e.Policy.InventoryMode(ctx, mode)
	var out Inventory
	if err == nil {
		err = convert(inv, &out)
	}
	return out, err
}
func (e *NativeEngine) BrowserInventory(ctx context.Context, mode string) (Inventory, error) {
	inv, err := e.Policy.BrowserInventory(ctx, mode)
	var out Inventory
	if err == nil {
		err = convert(inv, &out)
	}
	return out, err
}
func (e *NativeEngine) BrowseDirectory(ctx context.Context, path string) ([]Entry, error) {
	entries, err := e.Policy.BrowseDirectory(ctx, path)
	var out []Entry
	if err == nil {
		err = convert(entries, &out)
	}
	return out, err
}
func (e *NativeEngine) Snapshot(ctx context.Context, path string) (SnapshotResult, error) {
	value, err := e.Policy.Snapshot(ctx, path)
	return SnapshotResult{Paths: value.Paths, Skipped: value.Skipped}, err
}
func (e *NativeEngine) Highlight(ctx context.Context, mode, path string, request map[string]any) (HighlightResult, error) {
	var r *native.Request
	if request != nil {
		r = &native.Request{}
		if err := strictConvert(request, r); err != nil {
			return HighlightResult{}, err
		}
	}
	value, err := e.Policy.Highlight(ctx, mode, path, r)
	var out HighlightResult
	if err == nil {
		err = convert(value, &out)
	}
	return out, err
}
func nativeResult(p native.Plan) (PlanResult, error) {
	out := PlanResult{Direction: p.Direction, Unsafe: p.Gates.Unsafe, Desktop: p.Gates.Desktop, Backup: p.Gates.Backup, GatesKnown: true, Notices: p.Notices, Review: "Exact machine " + p.Machine.ID + " · nothing outside these actions is authorized."}
	if err := convert(p, &out.Plan); err != nil {
		return out, err
	}
	if err := convert(native.PlanDiffs(p), &out.Diffs); err != nil {
		return out, err
	}
	return out, nil
}
func (e *NativeEngine) Plan(ctx context.Context, request map[string]any) (PlanResult, error) {
	var r native.Request
	if err := strictConvert(request, &r); err != nil {
		return PlanResult{}, err
	}
	p, err := e.Policy.Plan(ctx, r)
	if err != nil {
		return PlanResult{}, err
	}
	return nativeResult(p)
}
func (e *NativeEngine) Preview(ctx context.Context, value any) (PlanResult, error) {
	var p native.Plan
	if err := strictConvert(value, &p); err != nil {
		return PlanResult{}, err
	}
	if _, err := e.Policy.Preview(ctx, p); err != nil {
		return PlanResult{}, err
	}
	return nativeResult(p)
}
func (e *NativeEngine) Apply(ctx context.Context, value any, id string, unsafe, desktop, backup bool) error {
	var p native.Plan
	if err := strictConvert(value, &p); err != nil {
		return err
	}
	return e.Policy.Apply(ctx, p, native.Consent{Approve: id, Unsafe: unsafe, Desktop: desktop, Backup: backup})
}
func (e *NativeEngine) Inspect(ctx context.Context, operation string) (string, error) {
	switch operation {
	case "status":
		inv, err := e.Policy.Status(ctx)
		if err != nil {
			return "", err
		}
		return formatStructuredStatus(inv), nil
	case "diff":
		diffs, err := e.Policy.Diff(ctx)
		if err != nil {
			return "", err
		}
		return formatStructuredDiff(diffs), nil
	case "scope":
		scope, err := e.Policy.ScopeShow(ctx)
		if err != nil {
			return "", err
		}
		return formatStructuredScope(scope), nil
	default:
		return "", fmt.Errorf("unknown inspection")
	}
}

func formatStructuredStatus(inv native.Inventory) string {
	var b strings.Builder
	b.WriteString("STATUS · Machine-Scoped Repository Inventory\n")
	fmt.Fprintf(&b, "Machines (%d): %s\n", len(inv.Machines), strings.Join(inv.Machines, ", "))
	if len(inv.Profiles) > 0 {
		names := make([]string, 0, len(inv.Profiles))
		for _, p := range inv.Profiles {
			names = append(names, fmt.Sprintf("%s (%d)", p.Name, len(p.Entries)))
		}
		fmt.Fprintf(&b, "Profiles (%d): %s\n", len(inv.Profiles), strings.Join(names, " · "))
	}
	managed, eligible, restricted := 0, 0, 0
	for _, e := range inv.Entries {
		switch e.Status {
		case "managed":
			managed++
		case "eligible":
			eligible++
		case "restricted":
			restricted++
		}
	}
	fmt.Fprintf(&b, "Entries: %d total (%d managed, %d eligible, %d restricted)\n", len(inv.Entries), managed, eligible, restricted)
	if managed > 0 {
		fmt.Fprintf(&b, "\nManaged (%d):\n", managed)
		for _, e := range inv.Entries {
			if e.Status != "managed" {
				continue
			}
			b.WriteString(statusLine(e) + "\n")
		}
	} else {
		b.WriteString("\nNo managed files.\n")
	}
	if eligible > 0 {
		b.WriteString("\nEligible:\n")
		for _, e := range inv.Entries {
			if e.Status == "eligible" {
				fmt.Fprintf(&b, "  • %s\n", e.Path)
			}
		}
	}
	if restricted > 0 {
		b.WriteString("\nRestricted:\n")
		for _, e := range inv.Entries {
			if e.Status == "restricted" {
				fmt.Fprintf(&b, "  • %s\n", e.Path)
			}
		}
	}
	return b.String()
}

// statusLine compresses one inventory entry to a state glyph plus path.
// Unchanged files stay one quiet line; only anomalies carry a suffix.
func statusLine(e native.InventoryEntry) string {
	switch e.TargetStatus {
	case "", "unchanged":
		return "  ✓ " + e.Path
	case "changed", "missing", "error":
		return "  ● " + e.Path + " · " + e.TargetStatus
	default:
		return "  ○ " + e.Path + " · " + e.TargetStatus
	}
}

func formatStructuredDiff(diffs []native.Diff) string {
	var b strings.Builder
	b.WriteString("DIFF · Importable Changes\n")
	if len(diffs) == 0 {
		b.WriteString("All approved target files match repository state. No pending changes.\n")
		return b.String()
	}
	fmt.Fprintf(&b, "%d file(s) with differences\n\n", len(diffs))
	for _, d := range diffs {
		if d.Summary != "" {
			fmt.Fprintf(&b, "● %s — %s\n", d.Path, d.Summary)
		} else {
			fmt.Fprintf(&b, "● %s\n", d.Path)
		}
		if d.Text != "" {
			b.WriteString(d.Text + "\n\n")
		} else {
			b.WriteString("(no text diff available)\n\n")
		}
	}
	return b.String()
}

func formatStructuredScope(a native.Adoption) string {
	var b strings.Builder
	b.WriteString("APPROVED SCOPE · Local Adoption Contract\n")
	fmt.Fprintf(&b, "Machine:  %s (%s)\n", a.Machine.ID, a.Machine.OS)
	fmt.Fprintf(&b, "Approval: %s\n", a.Profile)
	if len(a.Profiles) > 0 {
		fmt.Fprintf(&b, "Profiles: %s\n", strings.Join(a.Profiles, ", "))
	}
	fmt.Fprintf(&b, "\nApproved Entries (%d):\n", len(a.Entries))
	if len(a.Entries) == 0 {
		b.WriteString("Nothing approved yet. Adopt files from the menu to build approved scope.\n")
	}
	keys := make([]string, 0, len(a.Entries))
	for k := range a.Entries {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	for _, id := range keys {
		e := a.Entries[id]
		fmt.Fprintf(&b, "  • %-24s → %-24s [%s, %s]\n", id, e.Destination, e.Portability, e.Impact)
	}
	if len(a.Deselected) > 0 {
		fmt.Fprintf(&b, "\nDeselected Entries (%d): %s\n", len(a.Deselected), strings.Join(a.Deselected, ", "))
	}
	if len(a.Customs) > 0 {
		fmt.Fprintf(&b, "\nCustom Metadata Edits (%d):\n", len(a.Customs))
		for id, c := range a.Customs {
			fmt.Fprintf(&b, "  • %s: %v\n", id, c)
		}
	}
	return b.String()
}
func (e *NativeEngine) Remote(ctx context.Context, operation string, confirm bool) error {
	switch operation {
	case "pull":
		return e.Policy.Pull(ctx, confirm)
	case "push":
		return e.Policy.Push(ctx, confirm)
	}
	return fmt.Errorf("unknown remote operation")
}

// RecoveryPending reports interrupted-operation state without performing
// recovery. The returning adapter refuses pull/push while it reports true.
func (e *NativeEngine) RecoveryPending() (bool, error) {
	if e == nil || e.Policy == nil {
		return false, fmt.Errorf("native policy unavailable")
	}
	_, err := e.Policy.Recovery()
	if err == nil {
		return true, nil
	}
	if errors.Is(err, os.ErrNotExist) {
		return false, nil
	}
	return false, err
}

// ConfigPaths exposes source/home/state/tracker paths for the Settings
// pane. Read-only; setup engines inherit it through embedding.
func (e *NativeEngine) ConfigPaths() (source, home, state, tracker string) {
	if e == nil || e.Policy == nil {
		return "", "", "", ""
	}
	c := e.Policy.Config()
	return c.Source, c.Home, c.State, c.TrackerConfig
}

// DashboardStatus summarizes tracked state for returning-home. Read-only:
// single Status pass; TargetStatus already carries changed/missing/error so a
// second full Diff content pass (which re-reads every file) is skipped. The
// same Status/Diff reads the inspect screens show on demand.
func (e *NativeEngine) DashboardStatus(ctx context.Context) (DashboardData, error) {
	var out DashboardData
	inv, err := e.Policy.Status(ctx)
	if err != nil {
		return out, err
	}
	if err := convert(inv.Machine, &out.Machine); err != nil {
		return out, err
	}
	for _, entry := range inv.Entries {
		switch entry.Status {
		case "managed":
			dest := entry.Destination
			if dest == "" {
				dest = entry.Path
			}
			out.Managed = append(out.Managed, DashboardEntry{Path: entry.Path, Dest: dest, State: entry.TargetStatus, Portability: inputValue(entry.Metadata["portability"]), Impact: inputValue(entry.Metadata["impact"]), Assignment: assignmentDescription(entry.Metadata)})
			switch entry.TargetStatus {
			case "changed", "missing", "error":
				out.Diffs++
			}
		case "eligible":
			out.Adoptable++
		}
	}
	sort.Slice(out.Managed, func(i, j int) bool { return out.Managed[i].Path < out.Managed[j].Path })
	return out, nil
}

package tui

import (
	"context"
	"encoding/json"
	"fmt"

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
	var result any
	var err error
	switch operation {
	case "status":
		result, err = e.Policy.Status(ctx)
	case "diff":
		result, err = e.Policy.Diff(ctx)
	case "scope":
		result, err = e.Policy.ScopeShow(ctx)
	default:
		return "", fmt.Errorf("unknown inspection")
	}
	if err != nil {
		return "", err
	}
	b, err := json.MarshalIndent(result, "", "  ")
	return string(b), err
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

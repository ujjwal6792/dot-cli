package browser

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"os/exec"
)

// Client is the only boundary between the terminal UI and shared policy engine.
// It sends explicit checkout paths and never derives approval from inventory.
type Client struct {
	Executable string
	Engine     string
	Source     string
	Home       string
	Python     string
}

type Entry struct {
	Path         string         `json:"path"`
	ID           string         `json:"id,omitempty"`
	Kind         string         `json:"kind"`
	Status       string         `json:"status"`
	Destination  string         `json:"destination,omitempty"`
	Metadata     map[string]any `json:"metadata,omitempty"`
	LogicalID    string         `json:"logical_id,omitempty"`
	Tracking     map[string]any `json:"tracking,omitempty"`
	OSStatus     string         `json:"os_status,omitempty"`
	TargetStatus string         `json:"target_status,omitempty"`
	InScope      bool           `json:"in_scope,omitempty"`
	Approved     bool           `json:"approved,omitempty"`
	Open         bool           `json:"-"`
}

type Inventory struct {
	Machines []string  `json:"machines,omitempty"`
	OS       string    `json:"os,omitempty"`
	Entries  []Entry   `json:"entries"`
	Profiles []Profile `json:"profiles,omitempty"`
}
type Profile struct {
	Name    string   `json:"name"`
	Entries []string `json:"entries"`
}
type planResponse struct {
	Plan       map[string]any `json:"plan"`
	PlanID     string         `json:"plan_id"`
	Direction  string         `json:"direction"`
	Diffs      []Diff         `json:"diffs"`
	Unsafe     bool           `json:"unsafe"`
	Desktop    bool           `json:"desktop"`
	Backup     bool           `json:"backup"`
	GatesKnown bool           `json:"gates_known"`
	Notices    []string       `json:"notices,omitempty"`
	Summary    []string       `json:"summary,omitempty"`
	Review     string         `json:"review,omitempty"`
}

type envelope struct {
	Version             int    `json:"version"`
	Command             string `json:"command"`
	Engine              string `json:"engine"`
	Source              string `json:"source"`
	Home                string `json:"home,omitempty"`
	Request             any    `json:"request,omitempty"`
	Mode                string `json:"mode,omitempty"`
	Path                string `json:"path,omitempty"`
	Plan                any    `json:"plan,omitempty"`
	Approve             string `json:"approve,omitempty"`
	AckUnsafe           bool   `json:"ack_unsafe,omitempty"`
	AckDesktop          bool   `json:"ack_desktop,omitempty"`
	ReplaceSkillsBackup bool   `json:"replace_skills_backup,omitempty"`
}

type response struct {
	Version int             `json:"version"`
	OK      bool            `json:"ok"`
	Result  json.RawMessage `json:"result"`
	Error   string          `json:"error"`
}

func (c Client) call(ctx context.Context, command string, request, plan any, approve string, unsafe, desktop, backup bool, out any) error {
	if c.Executable == "" || c.Engine == "" || c.Source == "" || c.Home == "" {
		return errors.New("engine executable, --engine, --source, and --home are required")
	}
	payload := envelope{Version: 1, Command: command, Engine: c.Engine, Source: c.Source, Home: c.Home, Request: request, Plan: plan, Approve: approve, AckUnsafe: unsafe, AckDesktop: desktop, ReplaceSkillsBackup: backup}
	if fields, ok := request.(map[string]any); ok {
		if command == "inventory" {
			payload.Mode, _ = fields["mode"].(string)
			payload.Request = nil
		}
		if command == "snapshot" || command == "highlight" {
			payload.Path, _ = fields["path"].(string)
			if command == "snapshot" {
				payload.Request = nil
			} else {
				payload.Mode, _ = fields["mode"].(string)
				payload.Request = fields["request"]
			}
		}
	}
	body, err := json.Marshal(payload)
	if err != nil {
		return err
	}
	python := c.Python
	if python == "" {
		python = "python3"
	}
	cmd := exec.CommandContext(ctx, python, "-B", c.Executable)
	if c.Home != "" {
		cmd.Env = append(os.Environ(), "HOME="+c.Home)
	}
	cmd.Stdin = bytes.NewReader(append(body, '\n'))
	var stdout, stderr bytes.Buffer
	cmd.Stdout, cmd.Stderr = &stdout, &stderr
	runErr := cmd.Run()
	var result response
	if err := json.Unmarshal(stdout.Bytes(), &result); err != nil {
		if runErr != nil {
			return fmt.Errorf("engine adapter did not return JSON (%s): %w", python, runErr)
		}
		return fmt.Errorf("invalid engine response: %w", err)
	}
	if result.Version != 1 {
		return fmt.Errorf("unsupported engine response version %d (expected 1)", result.Version)
	}
	if !result.OK {
		if result.Error == "" {
			result.Error = "engine rejected request"
		}
		return errors.New(result.Error)
	}
	if runErr != nil {
		return fmt.Errorf("engine adapter returned success with a failing process status (%s): %w", python, runErr)
	}
	return json.Unmarshal(result.Result, out)
}

func (c Client) Inventory(ctx context.Context) (Inventory, error) {
	var result Inventory
	err := c.call(ctx, "inventory", nil, nil, "", false, false, false, &result)
	return result, err
}

func (c Client) InventoryMode(ctx context.Context, mode string) (Inventory, error) {
	var result Inventory
	err := c.call(ctx, "inventory", map[string]any{"mode": mode}, nil, "", false, false, false, &result)
	return result, err
}

type SnapshotResult struct {
	Paths   []string `json:"paths"`
	Skipped int      `json:"skipped"`
}
type HighlightResult struct {
	Status    string `json:"status"`
	Direction string `json:"direction"`
	Summary   string `json:"summary"`
	Text      string `json:"text"`
	Review    string `json:"review"`
	Diffs     []Diff `json:"diffs"`
}

func (c Client) Snapshot(ctx context.Context, path string) (SnapshotResult, error) {
	var result SnapshotResult
	err := c.call(ctx, "snapshot", map[string]any{"path": path}, nil, "", false, false, false, &result)
	return result, err
}
func (c Client) Highlight(ctx context.Context, mode, path string, request map[string]any) (HighlightResult, error) {
	var result HighlightResult
	fields := map[string]any{"path": path, "mode": mode}
	if request != nil {
		fields["request"] = request
	}
	err := c.call(ctx, "highlight", fields, nil, "", false, false, false, &result)
	return result, err
}

func (c Client) Plan(ctx context.Context, request map[string]any) (PlanResult, error) {
	var result planResponse
	err := c.call(ctx, "plan", request, nil, "", false, false, false, &result)
	return PlanResult{Plan: result.Plan, Direction: result.Direction, Diffs: result.Diffs, Unsafe: result.Unsafe, Desktop: result.Desktop, Backup: result.Backup, GatesKnown: result.GatesKnown, Notices: result.Notices, Summary: result.Summary, Review: result.Review}, err
}

func (c Client) Preview(ctx context.Context, plan any) (PlanResult, error) {
	var result planResponse
	err := c.call(ctx, "preview", nil, plan, "", false, false, false, &result)
	return PlanResult{Plan: map[string]any{"id": result.PlanID}, Direction: result.Direction, Diffs: result.Diffs, Unsafe: result.Unsafe, Desktop: result.Desktop, Backup: result.Backup, GatesKnown: result.GatesKnown, Notices: result.Notices, Summary: result.Summary, Review: result.Review}, err
}

func (c Client) Apply(ctx context.Context, plan any, approve string, unsafe, desktop, backup bool) error {
	var result any
	return c.call(ctx, "apply", nil, plan, approve, unsafe, desktop, backup, &result)
}

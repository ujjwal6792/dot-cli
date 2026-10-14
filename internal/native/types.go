// Package native owns offline configuration policy. Setup and command routing
// use these same types; no private executable is ever a policy adapter.
package native

import "os"

type Config struct {
	Source, Home, State, TrackerConfig, Git, Chezmoi, OS string
}
type Machine struct {
	ID string `json:"id"`
	OS string `json:"os"`
}
type VersionRange struct {
	Min int `json:"min"`
	Max int `json:"max"`
}
type Contract struct {
	Protocol         int          `json:"protocol"`
	EngineAPI        VersionRange `json:"engine_api"`
	ManifestVersion  int          `json:"manifest_version"`
	ApprovalVersion  int          `json:"approval_version"`
	Manifest         string       `json:"manifest"`
	HomeSource       string       `json:"home_source"`
	RequiredAdapters []string     `json:"required_adapters"`
}
type Assignment struct {
	Include []string `json:"include"`
	Exclude []string `json:"exclude"`
}
type Entry struct {
	Source       string            `json:"source"`
	Destination  string            `json:"destination"`
	OS           []string          `json:"os"`
	Portability  string            `json:"portability"`
	Impact       string            `json:"impact"`
	Machines     *Assignment       `json:"machines,omitempty"`
	Removals     []string          `json:"removals,omitempty"`
	Hooks        []string          `json:"hooks,omitempty"`
	Tools        []string          `json:"tools,omitempty"`
	Warnings     []string          `json:"warnings,omitempty"`
	Integrations []string          `json:"integrations,omitempty"`
	Requires     []string          `json:"requires,omitempty"`
	Notices      []string          `json:"notices,omitempty"`
	GitKeys      []string          `json:"git_keys,omitempty"`
	GitValues    map[string]string `json:"git_values,omitempty"`
	GitSource    string            `json:"git_source,omitempty"`
	Template     string            `json:"template,omitempty"`
	LogicalID    string            `json:"logical_id,omitempty"`
}
type Logical struct {
	Destinations map[string]string         `json:"destinations"`
	Sources      map[string]string         `json:"sources"`
	Common       map[string]any            `json:"common"`
	Machine      map[string]map[string]any `json:"machine"`
	OSMetadata   map[string]map[string]any `json:"os_metadata"`
	Machines     *Assignment               `json:"machines,omitempty"`
}
type Manifest struct {
	Version  int                    `json:"version"`
	Entries  map[string]Entry       `json:"entries"`
	Profiles map[string][]string    `json:"profiles"`
	Machines map[string]MachineInfo `json:"machines,omitempty"`
	Tracking map[string]Logical     `json:"tracking,omitempty"`
}
type MachineInfo struct {
	Label string `json:"label,omitempty"`
	OS    string `json:"os"`
}
type Adoption struct {
	Version    int                       `json:"version"`
	Profile    string                    `json:"profile"`
	Entries    map[string]Entry          `json:"entries"`
	Machine    Machine                   `json:"machine"`
	Profiles   []string                  `json:"profiles,omitempty"`
	Deselected []string                  `json:"deselected,omitempty"`
	Customs    map[string]map[string]any `json:"customs,omitempty"`
}
type Selection struct {
	Path string `json:"path"`
	Entry
	Logical      string                    `json:"logical,omitempty"`
	Destinations map[string]string         `json:"destinations,omitempty"`
	Common       map[string]any            `json:"common,omitempty"`
	Machine      map[string]map[string]any `json:"machine,omitempty"`
	OSMetadata   map[string]map[string]any `json:"os_metadata,omitempty"`
}
type Request struct {
	Version  int      `json:"version"`
	Mode     string   `json:"mode"`
	Profile  string   `json:"profile"`
	Profiles []string `json:"profiles,omitempty"`
	// nil means profile resolution; an empty nonnil slice is an exact deselection.
	Entries    []string                  `json:"entries"`
	Selections []Selection               `json:"selections,omitempty"`
	Metadata   map[string]map[string]any `json:"metadata,omitempty"`
}
type FileState struct {
	Exists bool        `json:"exists"`
	Data   []byte      `json:"data,omitempty"`
	Mode   os.FileMode `json:"mode"`
	Hash   string      `json:"hash,omitempty"`
}
type Action struct {
	Root         string            `json:"root"`
	Path         string            `json:"path"`
	Before       FileState         `json:"before"`
	After        FileState         `json:"after"`
	BackupPath   string            `json:"backup_path,omitempty"`
	BackupBefore FileState         `json:"backup_before,omitempty"`
	GitValues    map[string]string `json:"git_values,omitempty"`
}
type Gates struct {
	Unsafe  bool `json:"unsafe"`
	Desktop bool `json:"desktop"`
	Backup  bool `json:"backup"`
}
type Plan struct {
	Version     int              `json:"version"`
	ID          string           `json:"id"`
	Request     Request          `json:"request"`
	Machine     Machine          `json:"machine"`
	Selected    map[string]Entry `json:"selected"`
	Actions     []Action         `json:"actions"`
	Gates       Gates            `json:"gates"`
	Fingerprint string           `json:"fingerprint"`
	Direction   string           `json:"direction"`
	Notices     []string         `json:"notices,omitempty"`
}
type Consent struct {
	Approve                 string
	Unsafe, Desktop, Backup bool
}
type Diff struct {
	Path    string `json:"path"`
	Status  string `json:"status"`
	Summary string `json:"summary"`
	Text    string `json:"text"`
}
type InventoryEntry struct {
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
	InScope      bool           `json:"in_scope"`
	Approved     bool           `json:"approved"`
}
type Profile struct {
	Name    string   `json:"name"`
	Entries []string `json:"entries"`
}
type Inventory struct {
	Machines []string         `json:"machines"`
	OS       string           `json:"os"`
	Machine  Machine          `json:"machine"`
	Entries  []InventoryEntry `json:"entries"`
	Profiles []Profile        `json:"profiles"`
}

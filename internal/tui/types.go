package tui

// These values cross the in-process UI/engine boundary. The native engine is
// implemented by issues #16 and #17; there is no external adapter protocol.
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
	Machine  Machine   `json:"machine"`
	Machines []string  `json:"machines,omitempty"`
	OS       string    `json:"os,omitempty"`
	Entries  []Entry   `json:"entries"`
	Profiles []Profile `json:"profiles,omitempty"`
}

type Profile struct {
	Name    string   `json:"name"`
	Entries []string `json:"entries"`
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

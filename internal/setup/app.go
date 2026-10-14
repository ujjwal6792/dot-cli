package setup

import (
	"context"

	tea "charm.land/bubbletea/v2"
	"github.com/ujjwal6792/dots-cli/internal/tui"
)

var _ tui.Engine = (*Engine)(nil)
var _ tui.SourceEngine = (*Engine)(nil)
var _ tui.SourceTerminalEngine = (*Engine)(nil)
var _ tui.MachineEngine = (*Engine)(nil)
var _ tui.SetupEngine = (*Engine)(nil)

// NewApp is the common bare-dot / dot-init entry point for #18. Construction
// and selection are read-only. Incomplete setup offers explicit recovery;
// complete returning users retain #16's navigation and approval gates.
func NewApp(ctx context.Context, o Options) (tea.Model, *Engine, error) {
	e, err := New(o)
	if err != nil {
		return nil, nil, err
	}
	if e.Complete(ctx) == nil {
		return tui.NewReturningModelContext(ctx, e), e, nil
	}
	return tui.NewInitModelContext(ctx, e), e, nil
}

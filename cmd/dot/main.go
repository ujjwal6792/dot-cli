// Command dot is the single native application entry point.
package main

import (
	"context"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"os"
	"os/exec"
	"os/signal"
	"strings"
	"syscall"

	tea "charm.land/bubbletea/v2"
	"github.com/ujjwal6792/dots-cli/internal/distribution"
	"github.com/ujjwal6792/dots-cli/internal/native"
	"github.com/ujjwal6792/dots-cli/internal/setup"
)

var version = "development"
var buildIdentity = "development"

func main() {
	ctx, cancel := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer cancel()
	os.Exit(run(ctx, os.Args[1:], os.Stdin, os.Stdout, os.Stderr))
}

func run(ctx context.Context, args []string, in io.Reader, out, errOut io.Writer) int {
	if version != "development" && buildIdentity != distribution.Identity(version) {
		fmt.Fprintln(errOut, "dot: release build identity mismatch")
		return 1
	}
	if wantsHelp(args) {
		help(out, args)
		return 0
	}
	if len(args) == 1 && (args[0] == "--version" || args[0] == "-v" || args[0] == "version") {
		fmt.Fprintln(out, "dot "+version)
		return 0
	}
	err := dispatch(ctx, args, in, out, errOut)
	if err == nil {
		return 0
	}
	fmt.Fprintln(errOut, "dot:", err)
	if ctx.Err() != nil || errors.Is(err, context.Canceled) || errors.Is(err, tea.ErrInterrupted) {
		return 130
	}
	var usage usageError
	if errors.As(err, &usage) {
		return 2
	}
	return 1
}

func wantsHelp(args []string) bool {
	if len(args) > 0 && args[0] == "help" {
		return true
	}
	values := map[string]bool{"home": true, "source": true, "config": true, "state-dir": true, "git": true, "chezmoi": true, "name": true, "email": true, "format": true, "mode": true, "action": true, "url": true, "machine": true, "tracker-apply": true, "request": true, "plan": true, "approve": true, "root": true, "archive": true, "sha256": true}
	for i := 0; i < len(args); i++ {
		a := args[i]
		if a == "--" {
			break
		}
		if a == "--help" || a == "-h" {
			return true
		}
		if strings.HasPrefix(a, "--") && !strings.Contains(a, "=") && values[strings.TrimPrefix(a, "--")] {
			i++
		}
	}
	return false
}

type usageError string

func (e usageError) Error() string { return string(e) }
func dispatch(ctx context.Context, args []string, in io.Reader, out, errOut io.Writer) error {
	if len(args) > 0 {
		switch args[0] {
		case "-u", "--uninstall":
			args = append([]string{"uninstall"}, args[1:]...)
		case "--health", "doctor":
			args = append([]string{"health"}, args[1:]...)
		}
	}
	verb := "init"
	if len(args) > 0 && !strings.HasPrefix(args[0], "--") {
		verb, args = args[0], args[1:]
	}
	if len(args) > 0 && args[0] == "--health" {
		verb, args = "health", args[1:]
	}
	if len(args) > 0 && (args[0] == "--uninstall" || args[0] == "-u") {
		verb, args = "uninstall", args[1:]
	}
	sub := ""
	if verb == "intent" || verb == "scope" || verb == "contract" {
		if len(args) == 0 {
			return usageError("subcommand required; use --help")
		}
		sub, args = args[0], args[1:]
	}
	if verb == "intent" && (sub == "rounds" || sub == "pick") {
		verb = "browse"
		sub = ""
	}
	switch verb {
	case "init", "browse", "status", "diff", "scope", "intent", "contract", "pull", "push", "recovery", "recover", "health", "install", "self-update", "uninstall", "install-recover":
	default:
		return usageError("unknown command " + verb + "; use --help")
	}
	if verb == "install" || verb == "self-update" || verb == "uninstall" || verb == "install-recover" {
		return installation(verb, args, out, errOut)
	}
	fs := flag.NewFlagSet(verb, flag.ContinueOnError)
	fs.SetOutput(errOut)
	var o setup.Options
	o.BindFlags(fs)
	format := fs.String("format", "json", "JSON output")
	mode := fs.String("mode", "adopt", "inventory mode: track, adopt, untrack")
	check := fs.Bool("check", false, "read-only init verdict")
	action := fs.String("action", "", "new or clone")
	url := fs.String("url", "", "credential-free source URL")
	machine := fs.String("machine", "", "save machine identity")
	trackerPlan := fs.Bool("tracker-plan", false, "review tracker")
	trackerApply := fs.String("tracker-apply", "", "tracker plan file")
	complete := fs.Bool("complete", false, "verify setup readiness")
	requestFile := fs.String("request", "", "strict request JSON file or -")
	planFile := fs.String("plan", "", "strict plan JSON file or -")
	approve := fs.String("approve", "", "exact plan ID")
	yes := fs.Bool("yes", false, "explicit operation consent")
	unsafe := fs.Bool("ack-unsafe", false, "acknowledge unsafe")
	desktop := fs.Bool("ack-desktop", false, "acknowledge desktop effects")
	backup := fs.Bool("ack-backup", false, "acknowledge backup")
	if err := fs.Parse(args); err != nil {
		return usageError(err.Error())
	}
	if fs.NArg() != 0 {
		return usageError("unexpected arguments")
	}
	if *format != "json" {
		return usageError("--format must be json")
	}
	// Reject misplaced flags: an accepted flag must never silently claim an effect.
	allowed := map[string]bool{"home": true, "source": true, "config": true, "state-dir": true, "git": true, "chezmoi": true, "name": true, "email": true}
	if verb == "status" || verb == "diff" || verb == "scope" || verb == "contract" || verb == "health" || verb == "recovery" || (verb == "intent" && sub != "apply") || (verb == "init" && (*check || *trackerPlan || *complete)) {
		allowed["format"] = true
	}
	var extra []string
	switch verb {
	case "init":
		extra = []string{"check", "action", "url", "machine", "tracker-plan", "tracker-apply", "complete", "approve", "yes"}
	case "intent":
		switch sub {
		case "inventory":
			extra = []string{"mode"}
		case "plan":
			extra = []string{"request"}
		case "preview":
			extra = []string{"plan"}
		case "apply":
			extra = []string{"plan", "approve", "ack-unsafe", "ack-desktop", "ack-backup"}
		default:
			return usageError("unknown intent subcommand")
		}
	case "contract":
		if sub != "check" {
			return usageError("contract requires check")
		}
	case "scope":
		if sub != "show" {
			return usageError("scope requires show")
		}
	case "pull", "push":
		extra = []string{"yes"}
	case "recover":
		extra = []string{"approve"}
	}
	for _, k := range extra {
		allowed[k] = true
	}
	var invalid string
	fs.Visit(func(f *flag.Flag) {
		if !allowed[f.Name] {
			invalid = f.Name
		}
	})
	if invalid != "" {
		return usageError("unsupported --" + invalid + " for " + verb)
	}
	if verb == "health" {
		return health(ctx, o, out)
	}
	tools, err := distribution.Resolve(ctx, version, o.Git, o.Chezmoi)
	if err != nil {
		return err
	}
	o.Git, o.Chezmoi = tools.Git, tools.Chezmoi
	if verb != "init" && verb != "browse" {
		for _, tool := range []struct{ name, path string }{{"git", o.Git}, {"chezmoi", o.Chezmoi}} {
			if err := distribution.CheckTool(ctx, tool.path, tool.name); err != nil {
				return err
			}
		}
	}
	if verb == "init" || verb == "browse" {
		count := 0
		for _, enabled := range []bool{*check, *action != "", *machine != "", *trackerPlan, *trackerApply != "", *complete} {
			if enabled {
				count++
			}
		}
		if count > 1 {
			return usageError("choose one setup operation")
		}
		if verb == "init" && count > 0 {
			if *url != "" && *action == "" {
				return usageError("--url requires --action new|clone")
			}
			if (*approve != "" || *yes) && *trackerApply == "" {
				return usageError("--approve/--yes require --tracker-apply")
			}
		}
		if count == 0 {
			if *url != "" || *approve != "" || *yes {
				return usageError("setup action required")
			}
			model, _, err := setup.NewApp(ctx, o)
			if err != nil {
				return err
			}
			_, err = tea.NewProgram(model, tea.WithContext(ctx), tea.WithInput(in), tea.WithOutput(out)).Run()
			return err
		}
	}
	e, err := setup.New(o)
	if err != nil {
		return err
	}
	emit := func(v any) error { enc := json.NewEncoder(out); enc.SetIndent("", "  "); return enc.Encode(v) }
	switch verb {
	case "init":
		switch {
		case *check:
			v, err := e.Check(ctx)
			if err != nil {
				return err
			}
			return emit(map[string]string{"verdict": v})
		case *action != "":
			if *url == "" {
				return usageError("--url required")
			}
			return e.CreateSource(ctx, *action, *url)
		case *machine != "":
			return e.SaveMachine(ctx, *machine)
		case *trackerPlan:
			p, err := e.PlanTracker(ctx, o.Name, o.Email)
			if err != nil {
				return err
			}
			return emit(p)
		case *trackerApply != "":
			var p setup.TrackerPlan
			if err := readJSON(*trackerApply, in, &p); err != nil {
				return err
			}
			return e.ApplyTracker(ctx, p, *approve, *yes)
		case *complete:
			if err := e.Complete(ctx); err != nil {
				return err
			}
			return emit(map[string]bool{"ready": true})
		}
	case "status":
		v, err := e.Policy.Status(ctx)
		if err != nil {
			return err
		}
		return emit(v)
	case "diff":
		v, err := e.Policy.Diff(ctx)
		if err != nil {
			return err
		}
		a, err := e.Policy.LoadAdoption()
		if err != nil {
			return err
		}
		return emit(map[string]any{"machine": a.Machine, "diffs": v})
	case "scope":
		v, err := e.Policy.ScopeShow(ctx)
		if err != nil {
			return err
		}
		return emit(v)
	case "contract":
		v, err := e.Policy.CheckContract(ctx)
		if err != nil {
			return err
		}
		return emit(v)
	case "intent":
		if sub == "inventory" {
			v, err := e.Policy.InventoryMode(ctx, *mode)
			if err != nil {
				return err
			}
			return emit(v)
		}
		if sub == "plan" {
			var r native.Request
			if err := readJSON(*requestFile, in, &r); err != nil {
				return err
			}
			v, err := e.Policy.Plan(ctx, r)
			if err != nil {
				return err
			}
			return emit(v)
		}
		var p native.Plan
		if err := readJSON(*planFile, in, &p); err != nil {
			return err
		}
		if sub == "preview" {
			v, err := e.Preview(ctx, p)
			if err != nil {
				return err
			}
			return emit(v)
		}
		return e.Policy.Apply(ctx, p, native.Consent{Approve: *approve, Unsafe: *unsafe, Desktop: *desktop, Backup: *backup})
	case "pull":
		return e.Policy.Pull(ctx, *yes)
	case "push":
		return e.Policy.Push(ctx, *yes)
	case "recovery":
		v, err := e.Policy.Recovery()
		if err != nil {
			return err
		}
		return emit(v)
	case "recover":
		return e.Policy.Recover(ctx, *approve)
	}
	return usageError("unsupported operation")
}
func readJSON(path string, in io.Reader, v any) error {
	if path == "" {
		return usageError("JSON input file required")
	}
	var b []byte
	var err error
	if path != "-" {
		f, e := os.Open(path)
		if e != nil {
			return e
		}
		defer f.Close()
		in = f
	}
	b, err = io.ReadAll(io.LimitReader(in, (32<<20)+1))
	if err != nil {
		return err
	}
	if len(b) > 32<<20 {
		return fmt.Errorf("JSON input exceeds 32 MiB limit")
	}
	return native.Decode(b, v)
}
func health(ctx context.Context, o setup.Options, out io.Writer) error {
	// Does not load private state, source or config.
	report := map[string]string{"dot": version}
	tools, err := distribution.Resolve(ctx, version, o.Git, o.Chezmoi)
	if err != nil {
		return err
	}
	o.Git, o.Chezmoi = tools.Git, tools.Chezmoi
	for _, tool := range []struct{ name, path string }{{"git", o.Git}, {"chezmoi", o.Chezmoi}} {
		path := tool.path
		if path == "" {
			path = tool.name
		}
		p, err := exec.LookPath(path)
		if err != nil {
			return fmt.Errorf("%s missing: %w", tool.name, err)
		}
		if err := distribution.CheckTool(ctx, p, tool.name); err != nil {
			return err
		}
		b, err := exec.CommandContext(ctx, p, "--version").Output()
		if err != nil {
			return fmt.Errorf("%s incompatible: %w", tool.name, err)
		}
		report[tool.name] = p + ": " + strings.TrimSpace(string(b))
	}
	return json.NewEncoder(out).Encode(report)
}

func help(out io.Writer, args []string) {
	fmt.Fprintln(out, `dot — one native configuration app
Usage: dot [command] [options]
Bare dot and dot init open the same interactive app. Nothing is auto-applied.
Commands:
  init [--check]          Open setup; --check is offline and read-only
  browse                 Open the shared setup/returning-user app
  status, diff           Machine-scoped inspection (--format json)
  scope show             Machine-local approved scope (--format json)
  contract check         Offline protocol/manifest compatibility validation
  intent inventory       Machine-scoped inventory (--mode track|adopt|untrack)
  intent plan            Strict request JSON → exact plan JSON
  intent preview         Review fresh plan JSON without writes
  intent apply           Apply exact plan with explicit separate consents
  intent rounds, intent pick  Aliases of the same app (no second picker)
  pull, push             Explicit private Git operation (--yes)
  recovery               Inspect pending configuration recovery
  recover                Roll back exact configuration plan (--approve ID)
  health, --health, doctor Dependencies and installation diagnostics (no pin runner)
  install                Install verified native archive into owned root
  self-update            Promote verified archive; never updates private config
  uninstall, -u, --uninstall  Remove owned launch links; retain config and releases
  install-recover        Roll back an interrupted installation promotion
  help, -h, --help       This help or COMMAND --help
  version, -v, --version Version before private-config gates
Common: --home PATH --source PATH --config PATH --state-dir PATH
        --git PATH --chezmoi PATH --name NAME --email EMAIL
Paths must be clean absolute paths. Defaults are inside HOME; no checkout fallback.
Packaged --chezmoi overrides are refused: keep the verified release bundle intact.
Retired: add/remove, run, update NAME VERSION, layout/migrate, group scope runners,
personal/work --profile, --rounds and interpreter/browser/engine overrides.
Use reviewed Track/Adopt/untrack/update requests, not direct mutations or scripts.
Exit codes: 0 success, 1 operation/dependency failure, 2 invalid usage, 130 cancelled.`)
	joined := strings.Join(args, " ")
	switch {
	case strings.Contains(joined, "intent"):
		fmt.Fprintln(out, "intent inventory --mode track|adopt|untrack --format json\nintent plan --request FILE|- [--format json]\nintent preview --plan FILE|- [--format json]\nintent apply --plan FILE|- --approve EXACT_ID [--ack-unsafe] [--ack-desktop] [--ack-backup]\nRequest modes: track, adopt, untrack, update, metadata. Named profiles are JSON profile unions, not personal/work. Preview does not authorize apply; each required gate is independent.\nOptions follow the verb/subcommand; stdin JSON uses -. Update preserves exact prior enrollment.")
	case strings.Contains(joined, "init"):
		fmt.Fprintln(out, "init --check [common paths]: old-user/new-user contract verdict, no writes.\nScripted setup: init --action new|clone --url URL; init --machine ID; init --tracker-plan; init --tracker-apply FILE --approve ID --yes. Each is a separate explicit operation; no implicit adoption. init --complete verifies readiness.")
	case strings.Contains(joined, "install") || strings.Contains(joined, "self-update"):
		fmt.Fprintln(out, "install/self-update --root ABSOLUTE_ROOT --archive FILE --sha256 TRUSTED_SHA256 --yes\nuninstall --root ABSOLUTE_ROOT --yes\ninstall-recover --root ABSOLUTE_ROOT --yes\nUse a private empty root, never a Homebrew/pacman directory. Uninstall keeps all private data and immutable releases. Obtain checksum independently from the release publisher.")
	case strings.Contains(joined, "pull") || strings.Contains(joined, "push"):
		fmt.Fprintln(out, "pull|push --yes [common paths]. Pull is fast-forward-only, rejects dirty work, and never applies. Push sends existing commits only, never commits or forces.")
	case strings.Contains(joined, "recover"):
		fmt.Fprintln(out, "recovery [common paths]; recover --approve EXACT_PLAN_ID [common paths]. Conflicting post-interruption edits fail closed.")
	default:
		fmt.Fprintln(out, "Inspection: status|diff|scope show --format json [common paths]; all offline, no mutation. Health is available without private setup.")
	}
}

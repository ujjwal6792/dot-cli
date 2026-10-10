package main

import (
	"context"
	"flag"
	"fmt"
	"os"
	"path/filepath"

	browser "dot-browser"
)

func main() {
	engine := flag.String("engine", os.Getenv("DOT_ENGINE"), "absolute public dot engine directory")
	source := flag.String("source", os.Getenv("DOT_SOURCE"), "absolute private configuration repository")
	home := flag.String("home", os.Getenv("HOME"), "target HOME directory")
	mode := flag.String("mode", "track", "initial mode: track or adopt")
	rounds := flag.Bool("rounds", false, "offer optional Track/Adopt rounds; defaults to Finish")
	flag.Parse()
	if *engine == "" || *source == "" || *home == "" {
		fmt.Fprintln(os.Stderr, "dot-browser: --engine, --source, and --home (or DOT_ENGINE, DOT_SOURCE, and HOME) are required")
		os.Exit(2)
	}
	if *mode != "track" && *mode != "adopt" {
		fatal(fmt.Errorf("invalid mode %q", *mode))
	}
	enginePath, err := filepath.Abs(*engine)
	if err != nil {
		fatal(err)
	}
	sourcePath, err := filepath.Abs(*source)
	if err != nil {
		fatal(err)
	}
	homePath, err := filepath.Abs(*home)
	if err != nil {
		fatal(err)
	}
	client := browser.Client{Executable: filepath.Join(enginePath, "browser_api.py"), Engine: enginePath, Source: sourcePath, Home: homePath}
	if err := browser.Run(context.Background(), client, *rounds, *mode); err != nil {
		fatal(err)
	}
}

func fatal(err error) { fmt.Fprintln(os.Stderr, "dot-browser:", err); os.Exit(1) }

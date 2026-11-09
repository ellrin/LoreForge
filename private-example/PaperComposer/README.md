# Workspace template

Copy this whole folder into your **private** repository. It is the one place the
app reads and writes your personal material.

```
your-private-repo/
  profile/          phrase bank, vocabulary, rules, exemplars, equations
  publishers/       one folder per publisher: its .cls/.sty/.bst, a sample .tex,
                    and the template settings the app derives from them
  papers/           one folder per paper you are writing
    <paper>/
      main.tex
      sections/     one .tex per section, one sentence per line,
                    blank line between paragraphs
      refs.bib
      figures/
      .paperapp/    paragraph roles, ignored warnings, AI task log
  prompts/          your own prompt overrides (optional)
```

Point the app at the repository root, in either way:

- environment variable `PAPERCOMPOSER_WORKSPACE=/path/to/your-private-repo`
- or `workspace_path = "/path/to/your-private-repo"` in the app config file
  (`%APPDATA%\PaperComposer\config.toml` on Windows,
  `~/Library/Application Support/PaperComposer/config.toml` on macOS,
  `~/.config/PaperComposer/config.toml` on Linux)

The config file sits outside every repository, so the path is never committed.

Publisher files may carry their own licences. Keeping them in your private
repository avoids redistributing them.

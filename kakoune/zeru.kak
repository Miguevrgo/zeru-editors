# Zeru support for Kakoune: filetype, highlighting, indentation and, with
# kakoune-lsp, the language server built in this repository's server/.

declare-option -hidden str zeru_editors %sh{ dirname "$(dirname "$(readlink -f "$kak_source")")" }

# Detection
# ‾‾‾‾‾‾‾‾‾

hook global BufCreate .*[.]zr %{
    set-option buffer filetype zeru
}

# Initialization
# ‾‾‾‾‾‾‾‾‾‾‾‾‾‾

hook global WinSetOption filetype=zeru %<
    require-module zeru
    set-option window comment_line '//'
    set-option window indentwidth 4
    set-option window tabstop 4
    hook window ModeChange pop:insert:.* -group zeru-trim-indent zeru-trim-indent
    hook window InsertChar \n -group zeru-insert zeru-insert-on-new-line
    hook window InsertChar \n -group zeru-indent zeru-indent-on-new-line
    hook window InsertChar [}\])] -group zeru-indent zeru-indent-on-closing

    hook -once -always window WinSetOption filetype=.* %< remove-hooks window zeru-.+ >
>

hook -group zeru-highlight global WinSetOption filetype=zeru %{
    require-module zeru
    add-highlighter window/zeru ref zeru
    hook -once -always window WinSetOption filetype=.* %{ remove-highlighter window/zeru }
}

hook -group zeru-lsp global BufSetOption filetype=zeru %{
    try %{
        evaluate-commands %sh{
            printf 'set-option buffer lsp_servers %%{\n[zeru-lsp]\nroot_globs = [".git"]\ncommand = "%s/server/target/release/zeru-lsp"\n}\n' "$kak_opt_zeru_editors"
        }
    }
}

provide-module zeru %§

# Highlighters
# ‾‾‾‾‾‾‾‾‾‾‾‾

add-highlighter shared/zeru regions
add-highlighter shared/zeru/code default-region group

add-highlighter shared/zeru/comment region '//' '$' fill comment
add-highlighter shared/zeru/block_comment region '/\*' '\*/' fill comment

add-highlighter shared/zeru/string region '"' (?<!\\)(\\\\)*" group
add-highlighter shared/zeru/string/ fill string
add-highlighter shared/zeru/string/ regex '\\(?:[ntr0\\''"]|x[0-9a-fA-F]{2})' 0:meta

add-highlighter shared/zeru/raw_string region '`' '`' fill string

add-highlighter shared/zeru/code/ regex "'(?:\\\\(?:[ntr0\\\\'""]|x[0-9a-fA-F]{2})|[^'\\\\])'" 0:string

add-highlighter shared/zeru/code/ regex '#test\b' 0:meta
add-highlighter shared/zeru/code/ regex '\b(?:fn|var|const|pub|import|struct|enum|trait)\b' 0:keyword
add-highlighter shared/zeru/code/ regex '\b(?:if|else|while|for|in|return|break|continue|match|default|as|asm|volatile)\b' 0:keyword
add-highlighter shared/zeru/code/ regex '\b(?:i8|i16|i32|i64|isize|u8|u16|u32|u64|usize|f32|f64|bool|void|str)\b' 0:type
add-highlighter shared/zeru/code/ regex '\b[A-Z][a-z][A-Za-z0-9_]*\b' 0:type
add-highlighter shared/zeru/code/ regex '\b(?:true|false|None|Some|Ok|Err)\b' 0:value
add-highlighter shared/zeru/code/ regex '\bself\b' 0:variable
add-highlighter shared/zeru/code/ regex '\bfn\h+([A-Za-z_][A-Za-z0-9_]*)' 1:function
add-highlighter shared/zeru/code/ regex '\b(?:0x[0-9a-fA-F_]+|0b[01_]+|0o[0-7_]+|[0-9][0-9_]*(?:\.[0-9][0-9_]*)?(?:[eE][-+]?[0-9_]+)?)\b' 0:value
add-highlighter shared/zeru/code/ regex '\b(?:print|println|eprint|eprintln|assert|exit|zip|replace)\b(?=\()' 0:builtin

# Commands
# ‾‾‾‾‾‾‾‾

define-command -hidden zeru-trim-indent %{
    try %{ execute-keys -draft -itersel x s \h+$ <ret> d }
}

# A new line after a comment continues it with the same // and space.
define-command -hidden zeru-insert-on-new-line %<
    try %[
        evaluate-commands -draft -save-regs '/"' %[
            execute-keys -save-regs '' k x1s^\h*(//+\h*)<ret> y
            try %[
                execute-keys x<a-K>^\h*//+\h*$<ret> jxs^\h*<ret>P
            ] catch %[
                execute-keys d
            ]
        ]
        try %[ execute-keys -draft k x s\h+$<ret> d ]
    ]
>

# A new line keeps the indent above, one level more after an opening
# bracket; a closing bracket after the cursor goes back to its opening line's
# indent.
define-command -hidden zeru-indent-on-new-line %<
    evaluate-commands -draft -itersel %<
        try %< execute-keys -draft <semicolon> K <a-&> >
        try %<
            execute-keys -draft x <a-K> ^\h*// <ret>
            try %< execute-keys -draft k x <a-k> [{(\[]\h*$ <ret> j <a-gt> >
            try %< execute-keys -draft x <a-k> ^\h*[}\])] <ret> gh / [}\])] <ret> m <a-S> 1<a-&> >
        >
        try %< execute-keys -draft k : zeru-trim-indent <ret> >
    >
>

# A lone closing bracket goes to the indent of the line that opened it.
define-command -hidden zeru-indent-on-closing %<
    try %< execute-keys -draft -itersel <a-h> <a-k> ^\h*[}\])]$ <ret> h m <a-S> 1<a-&> >
>

§

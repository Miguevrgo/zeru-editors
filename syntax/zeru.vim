if exists("b:current_syntax")
  finish
endif

syntax keyword zeruKeyword fn var const pub if else import while return for in break continue as struct enum trait match default asm volatile
syntax keyword zeruBoolean true false
syntax keyword zeruConstant None Some Ok Err
syntax keyword zeruType i8 i16 i32 i64 isize u8 u16 u32 u64 usize f32 f64 bool void str
syntax match zeruType "\<[A-Z][a-z][A-Za-z0-9_]*\>"
syntax keyword zeruSelf self
syntax match zeruPreProc "#test\>"

syntax match zeruOperator "[-+*/%&|^<>!=]=*"
syntax match zeruOperator "&&"
syntax match zeruOperator "||"
syntax match zeruOperator "=>"
syntax match zeruOperator "::"
syntax match zeruOperator "\.\.=\="
syntax match zeruOperator "?"

syntax region zeruCommentLine start="//" end="$" contains=@Spell,zeruTodo
syntax region zeruCommentBlock start="/\*" end="\*/" contains=@Spell,zeruTodo
syntax keyword zeruTodo TODO FIXME XXX NOTE contained

syntax region zeruString start='"' skip='\\\\\|\\"' end='"' contains=zeruEscape,zeruFormat
syntax region zeruRawString start='`' end='`'
syntax match zeruChar "'\(\\x\x\x\|\\.\|[^'\\]\)'" contains=zeruEscape
syntax match zeruEscape /\\x\x\x\|\\./ contained
syntax match zeruFormat /{{\|}}\|{\(:[^{}"]*\)\?}/ contained

syntax match zeruNumber "\<\d[0-9_]*\>"
syntax match zeruNumber "\<0x[0-9a-fA-F_]\+\>"
syntax match zeruNumber "\<0b[01_]\+\>"
syntax match zeruNumber "\<0o[0-7_]\+\>"
syntax match zeruFloat "\<\d[0-9_]*\.\d[0-9_]*\([eE][+-]\=\d\+\)\=\>"
syntax match zeruFloat "\<\d[0-9_]*[eE][+-]\=\d\+\>"

highlight default link zeruKeyword Keyword
highlight default link zeruPreProc PreProc
highlight default link zeruBoolean Boolean
highlight default link zeruConstant Constant
highlight default link zeruType Type
highlight default link zeruCommentLine Comment
highlight default link zeruCommentBlock Comment
highlight default link zeruTodo Todo
highlight default link zeruString String
highlight default link zeruRawString String
highlight default link zeruChar Character
highlight default link zeruEscape Special
highlight default link zeruFormat Special
highlight default link zeruNumber Number
highlight default link zeruFloat Float
highlight default link zeruOperator Operator
highlight default link zeruSelf Identifier

let b:current_syntax = "zeru"

if exists("b:current_syntax")
  finish
endif

" Keywords
syntax keyword zeruKeyword fn var const if else import while return for in break continue as struct enum trait match default asm volatile
syntax keyword zeruBoolean true false None
syntax keyword zeruType i8 i16 i32 i64 u8 u16 u32 u64 usize f32 f64 bool str Array Vec
syntax keyword zeruSelf self

" Operators
syntax match zeruOperator "[-+*/%&|^<>!=]=*"
syntax match zeruOperator "&&"
syntax match zeruOperator "||"
syntax match zeruOperator "=>"
syntax match zeruOperator "::"

" Comments
syntax region zeruCommentLine start="//" end="$" contains=@Spell,zeruTodo
syntax region zeruCommentBlock start="/\*" end="\*/" contains=@Spell,zeruTodo
syntax keyword zeruTodo TODO FIXME XXX NOTE contained

" Strings
syntax region zeruString start='"' skip='\\"' end='"' contains=zeruEscape
syntax region zeruRawString start='`' end='`'
syntax match zeruEscape /\\./ contained

" Numbers
syntax match zeruNumber "\<\d\+\>"
syntax match zeruNumber "\<0x[0-9a-fA-F]\+\>"
syntax match zeruNumber "\<0b[01]\+\>"
syntax match zeruNumber "\<0o[0-7]\+\>"
syntax match zeruFloat "\<\d\+\.\d*\>"

highlight default link zeruKeyword Keyword
highlight default link zeruBoolean Boolean
highlight default link zeruType Type
highlight default link zeruCommentLine Comment
highlight default link zeruCommentBlock Comment
highlight default link zeruTodo Todo
highlight default link zeruString String
highlight default link zeruRawString String
highlight default link zeruEscape Special
highlight default link zeruNumber Number
highlight default link zeruFloat Float
highlight default link zeruOperator Operator
highlight default link zeruSelf Identifier

let b:current_syntax = "zeru"

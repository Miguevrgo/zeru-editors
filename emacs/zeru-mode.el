(defvar zeru-mode-syntax-table
  (let ((table (make-syntax-table)))
    ;; Comments
    (modify-syntax-entry ?/ ". 124b" table)
    (modify-syntax-entry ?* ". 23" table)
    (modify-syntax-entry ?\n "> b" table)
    ;; Strings
    (modify-syntax-entry ?\" "\"" table)
    (modify-syntax-entry ?\` "\"" table)
    table)
  "Syntax table for `zeru-mode'.")

(defconst zeru-font-lock-keywords
  (let* (
         (keywords '("fn" "var" "const" "if" "else" "import" "while" "return" "for" "in" "break" "continue" "as" "struct" "enum" "trait" "match" "default" "asm" "volatile"))
         (types '("i8" "i16" "i32" "i64" "u8" "u16" "u32" "u64" "usize" "f32" "f64" "bool" "str" "Array" "Vec"))
         (constants '("true" "false" "None" "self"))

         (keyword-regexp (regexp-opt keywords 'words))
         (type-regexp (regexp-opt types 'words))
         (constant-regexp (regexp-opt constants 'words)))
    `((,keyword-regexp . font-lock-keyword-face)
      (,type-regexp . font-lock-type-face)
      (,constant-regexp . font-lock-constant-face)))
  "Font lock keywords for `zeru-mode'.")

;;;###autoload
(define-derived-mode zeru-mode prog-mode "Zeru"
  "Major mode for editing Zeru source files."
  :syntax-table zeru-mode-syntax-table
  (setq-local font-lock-defaults '(zeru-font-lock-keywords)))

;;;###autoload
(add-to-list 'auto-mode-alist '("\\.zr\\'" . zeru-mode))

(provide 'zeru-mode)

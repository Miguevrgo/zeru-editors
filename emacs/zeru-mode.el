;;; zeru-mode.el --- Major mode for the Zeru programming language -*- lexical-binding: t -*-

(defvar zeru-mode-syntax-table
  (let ((table (make-syntax-table)))
    (modify-syntax-entry ?/ ". 124b" table)
    (modify-syntax-entry ?* ". 23" table)
    (modify-syntax-entry ?\n "> b" table)
    (modify-syntax-entry ?\" "\"" table)
    (modify-syntax-entry ?\` "\"" table)
    (modify-syntax-entry ?' "\"" table)
    (modify-syntax-entry ?_ "_" table)
    table)
  "Syntax table for `zeru-mode'.")

(defconst zeru-font-lock-keywords
  (let ((keywords '("fn" "var" "const" "pub" "if" "else" "import" "while" "return" "for" "in"
                    "break" "continue" "as" "struct" "enum" "trait" "match" "default" "asm"
                    "volatile"))
        (types '("i8" "i16" "i32" "i64" "isize" "u8" "u16" "u32" "u64" "usize" "f32" "f64"
                 "bool" "void" "str"))
        (constants '("true" "false" "None" "Some" "Ok" "Err" "self")))
    `((,(regexp-opt keywords 'symbols) . font-lock-keyword-face)
      (,(regexp-opt types 'symbols) . font-lock-type-face)
      ("\\_<[A-Z][a-z][A-Za-z0-9_]*\\_>" . font-lock-type-face)
      (,(regexp-opt constants 'symbols) . font-lock-constant-face)
      ("\\_<[0-9][0-9a-fA-FxXbBoO_]*\\(\\.[0-9][0-9_]*\\)?\\([eE][+-]?[0-9]+\\)?\\_>" . font-lock-constant-face)))
  "Font lock keywords for `zeru-mode'.")

(defun zeru-indent-line ()
  "Indent the current line one level per bracket open around it."
  (interactive)
  (let ((depth (save-excursion (car (syntax-ppss (line-beginning-position)))))
        (offset (- (current-column) (current-indentation))))
    (when (save-excursion (back-to-indentation) (looking-at "[]})]"))
      (setq depth (1- depth)))
    (indent-line-to (* 4 (max depth 0)))
    (when (> offset 0)
      (move-to-column (+ (current-indentation) offset)))))

;;;###autoload
(define-derived-mode zeru-mode prog-mode "Zeru"
  "Major mode for editing Zeru source files."
  :syntax-table zeru-mode-syntax-table
  (setq-local font-lock-defaults '(zeru-font-lock-keywords))
  (setq-local indent-line-function #'zeru-indent-line)
  (setq-local indent-tabs-mode nil)
  (setq-local comment-start "// ")
  (setq-local comment-end ""))

;;;###autoload
(add-to-list 'auto-mode-alist '("\\.zr\\'" . zeru-mode))

(provide 'zeru-mode)
;;; zeru-mode.el ends here

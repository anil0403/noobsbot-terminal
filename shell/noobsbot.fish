# noobsbot pretty prompt for fish (terminal.pretty_prompt), loaded with
# --init-command after config.fish. Fish marks prompts (OSC 133) on its own.
# Powerline segments in palette colors, so themes recolor it.

# Segment: an arrow from the previous one, or a rounded start.
function __noobsbot_seg --argument-names bg fg text
    if test -z "$__noobsbot_prev"
        printf '\e[38;5;%sm\e[48;5;%s;38;5;%sm' $bg $bg $fg
    else
        printf '\e[38;5;%s;48;5;%sm\e[38;5;%sm' $__noobsbot_prev $bg $fg
    end
    printf ' %s ' $text
    set -g __noobsbot_prev $bg
end

function fish_prompt
    set -l s $status
    set -g __noobsbot_prev
    set -l user $USER
    set -q SSH_CONNECTION; and set user $USER@(prompt_hostname)
    printf '\e[38;5;8m╭─'
    __noobsbot_seg 11 0 \uf007" $user"
    # Deep: …/last/two
    set -l dir (string replace -r "^$(string escape --style=regex -- $HOME)(?=/|\$)" '~' -- $PWD)
    set -l parts (string split / -- $dir)
    test (count $parts) -gt 3; and set dir "…/$parts[-2]/$parts[-1]"
    __noobsbot_seg 12 15 \uf07b" $dir"
    set -l branch (git symbolic-ref --short HEAD 2>/dev/null; or git rev-parse --short HEAD 2>/dev/null)
    if test -n "$branch"
        # ● when there's uncommitted work (no index lock: don't race the user's git).
        set -l bg 14
        test -n "$(GIT_OPTIONAL_LOCKS=0 git status --porcelain 2>/dev/null | head -c1)"; and set branch "$branch ●"; and set bg 208
        __noobsbot_seg $bg 0 \ue725" $branch"
    end
    # Failed: the exit code, and a red arrow.
    set -l arrow 2
    test $s -ne 0; and __noobsbot_seg 1 15 \uf00d" $s"; and set arrow 1
    printf '\e[0;38;5;%sm\ue0b4\n\e[38;5;8m╰─\e[38;5;%sm❯\e[0m ' $__noobsbot_prev $arrow
end

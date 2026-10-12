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
    __noobsbot_seg 11 0 $user
    set -l dir (path basename -- $PWD)
    test "$PWD" = "$HOME"; and set dir '~'
    __noobsbot_seg 208 15 " $dir"
    set -l branch (git symbolic-ref --short HEAD 2>/dev/null; or git rev-parse --short HEAD 2>/dev/null)
    test -n "$branch"; and __noobsbot_seg 14 0 " $branch"
    if test $s -eq 0
        __noobsbot_seg 12 15 
    else
        __noobsbot_seg 1 15 " $s"
    end
    printf '\e[0;38;5;%sm\e[0m ' $__noobsbot_prev
end

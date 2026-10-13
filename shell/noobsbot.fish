# noobsbot integration for fish, loaded with --init-command after config.fish.
# Fish marks prompts (OSC 133) and reports the directory on its own.

# Other machines won't know TERM=noobsbot.
if test "$TERM" = noobsbot; and not functions -q ssh
    function ssh --wraps ssh
        TERM=xterm-256color command ssh $argv
    end
end

# The pretty prompt (terminal.pretty_prompt): powerline segments in palette
# colors, so themes recolor it.
if test "$NOOBSBOT_PROMPT" = pretty
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

    # A blank line between prompts (not above the first), so their pills don't touch.
    # Before the prompt event, so it lands outside fish's prompt mark.
    function __noobsbot_gap --on-event fish_prompt
        set -q __noobsbot_gap; and echo
        set -g __noobsbot_gap
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
        if test -n "$branch"
            # ● when there's uncommitted work (no index lock: don't race the user's git).
            test -n "$(GIT_OPTIONAL_LOCKS=0 git status --porcelain 2>/dev/null | head -c1)"; and set branch "$branch ●"
            __noobsbot_seg 14 0 " $branch"
        end
        if test $s -eq 0
            __noobsbot_seg 12 15 
        else
            __noobsbot_seg 1 15 " $s"
        end
        printf '\e[0;38;5;%sm\e[0m ' $__noobsbot_prev
    end
end

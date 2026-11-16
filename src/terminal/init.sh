# Scout's terminal panel, for bash and zsh. Sourced after the user's own rc file, in the panel's shell only: the agent
# runs its commands elsewhere and keeps the shell's standard errors.
# A Windows command typed here (cls, ipconfig, del, Get-ChildItem...) that isn't a command on this computer runs as
# its equivalent, shown on a dim line first. Only when the rewrite is certain; otherwise it's a short "not a command".

__scout_lower() { printf '%s' "$1" | tr '[:upper:]' '[:lower:]'; }

# Sets __scout_cmd to the equivalent of "$@" (an array), or returns 1 when there isn't a certain one.
__scout_convert() {
  local name; name=$(__scout_lower "$1"); shift
  __scout_cmd=()
  case "$name" in
    cls|clear-host) [ $# -eq 0 ] && __scout_cmd=(clear) ;;
    ipconfig) [ $# -eq 0 ] || [ "$1" = "/all" ] || return 1
      if command -v ip >/dev/null 2>&1; then __scout_cmd=(ip addr); else __scout_cmd=(ifconfig); fi ;;
    ver|systeminfo) [ $# -eq 0 ] && __scout_cmd=(uname -a) ;;
    tasklist|get-process|gps) [ $# -eq 0 ] && __scout_cmd=(ps aux) ;;
    get-location|gl) [ $# -eq 0 ] && __scout_cmd=(pwd) ;;
    where|where.exe) [ $# -ge 1 ] && __scout_cmd=(which "$@") ;;
    del|erase)
      local force=() a files=()
      for a in "$@"; do
        case "$(__scout_lower "$a")" in /q|/f) force=(-f) ;; /*) return 1 ;; *) files+=("$a") ;; esac
      done
      [ ${#files[@]} -ge 1 ] && __scout_cmd=(rm "${force[@]}" -- "${files[@]}") ;;
    copy|move|ren|rename)
      local a files=() force=()
      for a in "$@"; do
        case "$(__scout_lower "$a")" in /y) force=(-f) ;; /*) return 1 ;; *) files+=("$a") ;; esac
      done
      [ ${#files[@]} -eq 2 ] || return 1
      case "$name" in copy) __scout_cmd=(cp "${force[@]}" -- "${files[@]}") ;; *) __scout_cmd=(mv "${force[@]}" -- "${files[@]}") ;; esac ;;
    md) [ $# -ge 1 ] && __scout_cmd=(mkdir -p -- "$@") ;;
    rd)
      local a dirs=() rec=0
      for a in "$@"; do
        case "$(__scout_lower "$a")" in /s) rec=1 ;; /q) ;; /*) return 1 ;; *) dirs+=("$a") ;; esac
      done
      [ ${#dirs[@]} -ge 1 ] || return 1
      if [ $rec = 1 ]; then __scout_cmd=(rm -rf -- "${dirs[@]}"); else __scout_cmd=(rmdir -- "${dirs[@]}"); fi ;;
    findstr)
      local a opts=() rest=()
      for a in "$@"; do
        case "$(__scout_lower "$a")" in /i) opts+=(-i) ;; /n) opts+=(-n) ;; /s) opts+=(-r) ;; /v) opts+=(-v) ;; /*) return 1 ;; *) rest+=("$a") ;; esac
      done
      [ ${#rest[@]} -ge 1 ] && __scout_cmd=(grep "${opts[@]}" -- "${rest[@]}") ;;
    taskkill)
      local force=() pid="" im="" next=""
      while [ $# -gt 0 ]; do
        case "$(__scout_lower "$1")" in
          /f) force=(-9) ;;
          /pid) shift; pid="$1" ;;
          /im) shift; im="$1" ;;
          *) return 1 ;;
        esac
        shift
      done
      if [ -n "$pid" ] && [ -z "$im" ]; then __scout_cmd=(kill "${force[@]}" "$pid")
      elif [ -n "$im" ] && [ -z "$pid" ]; then __scout_cmd=(pkill "${force[@]}" -x "${im%.exe}")
      else return 1; fi ;;
    get-childitem|gci|dir)
      local a rec="" paths=()
      for a in "$@"; do
        case "$(__scout_lower "$a")" in -recurse|-r) rec=R ;; -force) ;; -*|/*) return 1 ;; *) paths+=("$a") ;; esac
      done
      __scout_cmd=(ls "-la$rec" "${paths[@]}") ;;
    get-content|gc)
      local a files=()
      for a in "$@"; do case "$(__scout_lower "$a")" in -path) ;; -*) return 1 ;; *) files+=("$a") ;; esac; done
      [ ${#files[@]} -ge 1 ] && __scout_cmd=(cat -- "${files[@]}") ;;
    remove-item|ri)
      local a opts="" paths=()
      for a in "$@"; do
        case "$(__scout_lower "$a")" in -recurse|-r) opts="${opts}r" ;; -force) opts="${opts}f" ;; -path) ;; -*) return 1 ;; *) paths+=("$a") ;; esac
      done
      [ ${#paths[@]} -ge 1 ] || return 1
      if [ -n "$opts" ]; then __scout_cmd=(rm "-$opts" -- "${paths[@]}"); else __scout_cmd=(rm -- "${paths[@]}"); fi ;;
    copy-item|move-item)
      local a rec=() files=()
      for a in "$@"; do
        case "$(__scout_lower "$a")" in -recurse|-r) rec=(-r) ;; -force|-path|-destination) ;; -*) return 1 ;; *) files+=("$a") ;; esac
      done
      [ ${#files[@]} -eq 2 ] || return 1
      if [ "$name" = copy-item ]; then __scout_cmd=(cp "${rec[@]}" -- "${files[@]}"); else __scout_cmd=(mv -- "${files[@]}"); fi ;;
    new-item|ni)
      local a type="" paths=() want=""
      for a in "$@"; do
        if [ "$want" = type ]; then type=$(__scout_lower "$a"); want=""; continue; fi
        case "$(__scout_lower "$a")" in -itemtype|-type) want=type ;; -path|-force) ;; -*) return 1 ;; *) paths+=("$a") ;; esac
      done
      [ ${#paths[@]} -ge 1 ] || return 1
      case "$type" in directory) __scout_cmd=(mkdir -p -- "${paths[@]}") ;; file|"") __scout_cmd=(touch -- "${paths[@]}") ;; *) return 1 ;; esac ;;
    write-host|write-output)
      local a
      for a in "$@"; do case "$a" in -*) return 1 ;; esac; done
      __scout_cmd=(echo "$@") ;;
    *) return 1 ;;
  esac
  [ ${#__scout_cmd[@]} -gt 0 ]
}

__scout_not_found() {
  if [ -t 1 ] && __scout_convert "$@"; then
    printf '\033[38;2;122;122;122m\xe2\x86\xb3 %s\033[0m\n' "${__scout_cmd[*]}" >&2
    "${__scout_cmd[@]}"
    return $?
  fi
  if [ -n "$__scout_prev_handler" ]; then "$__scout_prev_handler" "$@"; return $?; fi
  printf '\033[38;2;232;100;100m%s isn'"'"'t a command here.\033[0m\n' "$1" >&2
  return 127
}

# Keep a handler the system already has (Ubuntu's suggests packages to install): it still answers what isn't converted.
__scout_prev_handler=""
if [ -n "$ZSH_VERSION" ]; then
  if (( $+functions[command_not_found_handler] )); then
    functions[__scout_prev_cnf]=$functions[command_not_found_handler]; __scout_prev_handler=__scout_prev_cnf
  fi
  command_not_found_handler() { __scout_not_found "$@"; }
else
  if declare -F command_not_found_handle >/dev/null; then
    eval "$(declare -f command_not_found_handle | sed '1s/command_not_found_handle/__scout_prev_cnf/')"; __scout_prev_handler=__scout_prev_cnf
  fi
  command_not_found_handle() { __scout_not_found "$@"; }
fi

# Where the prompt ends, for the panel's suggestions: an invisible OSC 133;B after it. Checked before every prompt, since
# prompt themes rebuild PS1 each time.
__scout_mark() {
  printf '\033]7;file://%s%s\007' "${HOSTNAME:-}" "$PWD"
  case "$PS1" in
    *'133;B'*) ;;
    *) if [ -n "$ZSH_VERSION" ]; then PS1="$PS1%{"$'\e]133;B\a'"%}"; else PS1="$PS1\[\e]133;B\a\]"; fi ;;
  esac
}
if [ -n "$ZSH_VERSION" ]; then
  precmd_functions+=(__scout_mark)
else
  PROMPT_COMMAND="${PROMPT_COMMAND:+$PROMPT_COMMAND;}__scout_mark"
fi

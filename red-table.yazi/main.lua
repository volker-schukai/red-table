--- @since 26.5.6

local M = {}

local DEFAULT_COMMAND = "red-table"
local MAX_OUTPUT_BYTES = 64 * 1024 * 1024
local MAX_OUTPUT_RECORDS = 1000000
local MAX_NOTIFICATION_BYTES = 4096

local snapshot = ya.sync(function(state)
  local cwd = cx.active.current.cwd
  local modern = cwd.spec ~= nil
  local virtual
  if modern then
    virtual = cwd.spec.is_virtual
  else
    virtual = cwd.scheme and cwd.scheme.is_virtual or false
  end
  return {
    command = state.command or DEFAULT_COMMAND,
    cwd = tostring(cwd),
    modern = modern,
    virtual = virtual,
  }
end)

local function bounded(value)
  local text = tostring(value or "unknown error"):gsub("%z", "?")
  if #text <= MAX_NOTIFICATION_BYTES then
    return text
  end
  return text:sub(1, MAX_NOTIFICATION_BYTES - 3) .. "..."
end

local function notify(content, level)
  ya.notify({
    title = "red-table",
    content = bounded(content),
    timeout = 7,
    level = level or "error",
  })
end

function M.setup(state, options)
  options = options or {}
  local command = options.command
  if command == nil then
    command = DEFAULT_COMMAND
  end
  if type(command) ~= "string" or command == "" then
    error("red-table.yazi: setup option `command` must be a nonempty string")
  end
  state.command = command
end

function M.run(command, cwd)
  return Command(command)
    :arg({ "--select", "--print0", "--", cwd })
    :stdin(Command.NULL)
    :stdout(Command.PIPED)
    :stderr(Command.PIPED)
    :output()
end

function M.parse_output(output, max_bytes, max_records)
  if type(output) ~= "string" then
    return nil, "stdout was not a byte string"
  end

  max_bytes = max_bytes or MAX_OUTPUT_BYTES
  max_records = max_records or MAX_OUTPUT_RECORDS
  if #output > max_bytes then
    return nil, string.format("selection output exceeds %d bytes", max_bytes)
  elseif output == "" then
    return {}
  end

  local urls, seen, start = {}, {}, 1
  while start <= #output do
    local stop = output:find("\0", start, true)
    if not stop then
      return nil, "selection output is not terminated by NUL"
    elseif stop == start then
      return nil, "selection output contains an empty path"
    elseif #urls >= max_records then
      return nil, string.format("selection output exceeds %d paths", max_records)
    end

    local path = output:sub(start, stop - 1)
    if seen[path] then
      return nil, "selection output contains a duplicate path"
    end
    seen[path] = true

    local url = Url(path)
    if not url.is_absolute then
      return nil, "selection output contains a relative path"
    end
    urls[#urls + 1] = url
    start = stop + 1
  end
  return urls
end

function M.apply(urls, modern)
  local files = {}
  local first = #urls > 0 and Url(urls[1]) or nil
  for _, url in ipairs(urls) do
    if modern then
      local file = fs.file(url)
      if not file then
        return false, string.format("cannot inspect selected path: %s", url)
      end
      files[#files + 1] = file
    else
      files[#files + 1] = Url(url)
    end
  end

  ya.emit("escape", { select = true })
  if #files == 0 then
    return true
  end

  files.state = "on"
  ya.emit("toggle_all", files)
  ya.emit("reveal", { first, raw = true })
  return true
end

function M.entry(_, _job)
  local state = snapshot()
  if state.virtual then
    return notify("Virtual Yazi directories are not supported; open a real directory first", "warn")
  elseif type(state.command) ~= "string" or state.command == "" then
    return notify("The configured red-table command is empty")
  end

  local permit_ok, permit = pcall(ui.hide)
  if not permit_ok then
    return notify("Cannot acquire the terminal: " .. bounded(permit))
  end

  local run_ok, output, command_error = pcall(M.run, state.command, state.cwd)
  local drop_ok, drop_error = pcall(function()
    permit:drop()
  end)
  if not drop_ok then
    return notify("Cannot restore Yazi's terminal: " .. bounded(drop_error))
  elseif not run_ok then
    return notify("red-table failed while running: " .. bounded(output))
  elseif not output then
    return notify("Cannot start red-table: " .. bounded(command_error))
  elseif output.status.code == 2 then
    return
  elseif not output.status.success then
    local detail = output.stderr ~= "" and output.stderr or "no diagnostic output"
    return notify(
      string.format("red-table exited with status %s: %s", output.status.code or "signal", detail)
    )
  end

  local parse_ok, urls, parse_error = pcall(M.parse_output, output.stdout)
  if not parse_ok then
    return notify("Cannot parse red-table selection: " .. bounded(urls))
  elseif not urls then
    return notify("Cannot parse red-table selection: " .. bounded(parse_error))
  end

  local apply_ok, applied, apply_error = pcall(M.apply, urls, state.modern)
  if not apply_ok then
    return notify("Cannot apply red-table selection: " .. bounded(applied))
  elseif not applied then
    return notify("Cannot apply red-table selection: " .. bounded(apply_error))
  end
end

return M

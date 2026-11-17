local passed = 0

local function assert_equal(actual, expected, message)
  if actual ~= expected then
    error(string.format("%s: expected %q, got %q", message, expected, actual), 2)
  end
end

local function assert_true(value, message)
  if not value then
    error(message, 2)
  end
end

local function test(name, body)
  local ok, err = xpcall(body, debug.traceback)
  if not ok then
    io.stderr:write(string.format("FAIL %s\n%s\n", name, err))
    os.exit(1)
  end
  passed = passed + 1
end

local function environment(options)
  options = options or {}
  local env = {
    command_error = options.command_error,
    emits = {},
    hidden = 0,
    notifications = {},
    output = options.output,
    state = {},
  }

  local url_mt = {
    __index = function(_, key)
      if options.forbid_legacy and key == "scheme" then
        error("modern URLs must not access deprecated scheme")
      end
    end,
    __tostring = function(value)
      return value.raw
    end,
  }
  _G.Url = function(value)
    if type(value) == "table" then
      value = value.raw
    end
    if options.url_error then
      error(options.url_error)
    end
    return setmetatable({ raw = value, is_absolute = value:sub(1, 1) == "/" }, url_mt)
  end

  local cwd = Url(options.cwd or "/photos")
  if options.modern == false then
    cwd.scheme = { is_virtual = options.virtual or false }
  else
    cwd.spec = { is_virtual = options.virtual or false }
  end
  _G.cx = { active = { current = { cwd = cwd } } }
  _G.ya = {
    emit = function(action, args)
      env.emits[#env.emits + 1] = { action = action, args = args }
    end,
    notify = function(notification)
      env.notifications[#env.notifications + 1] = notification
    end,
    sync = function(callback)
      return function(...)
        return callback(env.state, ...)
      end
    end,
  }
  _G.ui = {
    hide = function()
      if options.hide_error then
        error(options.hide_error)
      end
      env.hidden = env.hidden + 1
      return {
        drop = function()
          if options.drop_error then
            error(options.drop_error)
          end
          env.hidden = env.hidden - 1
        end,
      }
    end,
  }
  _G.fs = {
    file = function(url)
      env.file_calls = (env.file_calls or 0) + 1
      if options.file_error == tostring(url) then
        return nil
      end
      return { url = tostring(url) }
    end,
  }

  local command = { NULL = "null", PIPED = "piped" }
  setmetatable(command, {
    __call = function(_, program)
      local call = { program = program }
      env.command = call
      local builder = {}
      for _, method in ipairs({ "arg", "stdin", "stdout", "stderr" }) do
        builder[method] = function(self, value)
          call[method] = value
          return self
        end
      end
      builder.output = function()
        if options.run_error then
          error(options.run_error)
        end
        return env.output, env.command_error
      end
      return builder
    end,
  })
  _G.Command = command

  local chunk, load_error = loadfile("red-table.yazi/main.lua")
  assert(chunk, load_error)
  env.plugin = chunk()
  return env
end

local function output(code, stdout, stderr)
  return {
    status = { code = code, success = code == 0 },
    stdout = stdout or "",
    stderr = stderr or "",
  }
end

test("modern real directories never access the deprecated URL scheme", function()
  local env = environment({ forbid_legacy = true, output = output(0, "") })
  env.plugin:entry({})
  assert_equal(#env.notifications, 0, "no deprecated URL access or notification")
  assert_equal(env.hidden, 0, "terminal restored")
  assert_equal(env.emits[1].action, "escape", "selection confirmed")
end)

test("confirmed paths replace selection and reveal the first", function()
  local raw_second = "/photos/sub/two\n\255.png"
  local env = environment({ output = output(0, "/photos/one.png\0" .. raw_second .. "\0") })
  env.plugin.setup(env.state, { command = "/nix/store/red-table/bin/red-table" })
  env.plugin.entry(env.state, { args = {} })

  assert_equal(env.hidden, 0, "terminal permit count")
  assert_equal(env.command.program, "/nix/store/red-table/bin/red-table", "configured command")
  assert_equal(
    table.concat(env.command.arg, "|"),
    "--select|--print0|--|/photos",
    "command arguments"
  )
  assert_equal(env.command.stdin, Command.NULL, "command stdin")
  assert_equal(env.command.stdout, Command.PIPED, "command stdout")
  assert_equal(env.command.stderr, Command.PIPED, "command stderr")
  assert_equal(#env.emits, 3, "manager action count")
  assert_equal(env.emits[1].action, "escape", "first manager action")
  assert_true(env.emits[1].args.select, "old selection is cleared")
  assert_equal(env.emits[2].action, "toggle_all", "second manager action")
  assert_equal(env.emits[2].args.state, "on", "new selection state")
  assert_equal(env.emits[2].args[1].url, "/photos/one.png", "first selected file")
  assert_equal(env.emits[2].args[2].url, raw_second, "binary-safe second selected file")
  assert_equal(env.emits[3].action, "reveal", "third manager action")
  assert_equal(tostring(env.emits[3].args[1]), "/photos/one.png", "revealed file")
  assert_equal(#env.notifications, 0, "notifications")
end)

test("confirmed empty output clears selection", function()
  local env = environment({ output = output(0, "") })
  env.plugin.entry(env.state, { args = {} })
  assert_equal(env.hidden, 0, "terminal permit count")
  assert_equal(#env.emits, 1, "manager action count")
  assert_equal(env.emits[1].action, "escape", "clear selection action")
  assert_true(env.emits[1].args.select, "selection clear flag")
end)

test("Yazi 26.5.6 receives the legacy URL selection payload", function()
  local env = environment({
    modern = false,
    output = output(0, "/photos/one.png\0/photos/two.png\0"),
  })
  env.plugin.entry(env.state, { args = {} })

  assert_equal(env.hidden, 0, "terminal permit count")
  assert_equal(env.file_calls or 0, 0, "modern file conversions")
  assert_equal(env.emits[2].action, "toggle_all", "selection action")
  assert_equal(env.emits[2].args.state, "on", "selection state")
  assert_equal(tostring(env.emits[2].args[1]), "/photos/one.png", "first legacy URL")
  assert_equal(tostring(env.emits[2].args[2]), "/photos/two.png", "second legacy URL")
  assert_equal(tostring(env.emits[3].args[1]), "/photos/one.png", "revealed legacy URL")
end)

test("status two preserves Yazi state", function()
  local env = environment({ output = output(2, "", "cancelled") })
  env.plugin.entry(env.state, { args = {} })
  assert_equal(env.hidden, 0, "terminal permit count")
  assert_equal(#env.emits, 0, "manager actions")
  assert_equal(#env.notifications, 0, "notifications")
end)

test("child failures restore the terminal and notify", function()
  local env = environment({ output = output(1, "", "decoder failed") })
  env.plugin.entry(env.state, { args = {} })
  assert_equal(env.hidden, 0, "terminal permit count")
  assert_equal(#env.emits, 0, "manager actions")
  assert_equal(env.notifications[1].level, "error", "notification level")
  assert_true(env.notifications[1].content:find("decoder failed", 1, true), "child diagnostic")
end)

test("spawn failures restore the terminal and notify", function()
  local env = environment({ command_error = "not found" })
  env.plugin.entry(env.state, { args = {} })
  assert_equal(env.hidden, 0, "terminal permit count")
  assert_equal(#env.emits, 0, "manager actions")
  assert_true(env.notifications[1].content:find("not found", 1, true), "spawn diagnostic")
end)

test("unexpected command failures restore the terminal", function()
  local env = environment({ run_error = "runtime exploded" })
  env.plugin.entry(env.state, { args = {} })
  assert_equal(env.hidden, 0, "terminal permit count")
  assert_equal(#env.emits, 0, "manager actions")
  assert_true(env.notifications[1].content:find("runtime exploded", 1, true), "runtime diagnostic")
end)

test("malformed output never changes selection", function()
  local env = environment({ output = output(0, "/photos/unterminated.png") })
  env.plugin.entry(env.state, { args = {} })
  assert_equal(env.hidden, 0, "terminal permit count")
  assert_equal(#env.emits, 0, "manager actions")
  assert_true(env.notifications[1].content:find("terminated by NUL", 1, true), "parse diagnostic")
end)

test("virtual directories are rejected before terminal handoff", function()
  local env = environment({ virtual = true, output = output(0, "/photos/one.png\0") })
  env.plugin.entry(env.state, { args = {} })
  assert_equal(env.hidden, 0, "terminal permit count")
  assert_true(env.command == nil, "child was not started")
  assert_equal(env.notifications[1].level, "warn", "notification level")
end)

test("terminal acquisition failure does not start the child", function()
  local env = environment({ hide_error = "terminal busy", output = output(0, "/photos/one.png\0") })
  env.plugin.entry(env.state, { args = {} })
  assert_equal(env.hidden, 0, "terminal permit count")
  assert_true(env.command == nil, "child was not started")
  assert_true(env.notifications[1].content:find("terminal busy", 1, true), "terminal diagnostic")
end)

test("unresolvable selected files do not clear the old selection", function()
  local env = environment({
    file_error = "/photos/missing.png",
    output = output(0, "/photos/missing.png\0"),
  })
  env.plugin.entry(env.state, { args = {} })
  assert_equal(env.hidden, 0, "terminal permit count")
  assert_equal(#env.emits, 0, "manager actions")
  assert_true(env.notifications[1].content:find("missing.png", 1, true), "selected-file diagnostic")
end)

test("strict parser rejects unsafe result framing", function()
  local env = environment()
  local cases = {
    { "relative.png\0", "relative path" },
    { "/a\0/a\0", "duplicate path" },
    { "/a\0\0", "empty path" },
  }
  for _, case in ipairs(cases) do
    local urls, err = env.plugin.parse_output(case[1])
    assert_true(urls == nil, "malformed output accepted")
    assert_true(err:find(case[2], 1, true), "unexpected parser diagnostic")
  end

  local urls, err = env.plugin.parse_output("/a\0/b\0", 100, 1)
  assert_true(urls == nil, "record limit was ignored")
  assert_true(err:find("exceeds 1 paths", 1, true), "record limit diagnostic")
  urls, err = env.plugin.parse_output("/long\0", 2, 10)
  assert_true(urls == nil, "byte limit was ignored")
  assert_true(err:find("exceeds 2 bytes", 1, true), "byte limit diagnostic")
end)

test("setup rejects an empty executable", function()
  local env = environment()
  local ok, err = pcall(env.plugin.setup, env.state, { command = "" })
  assert_true(not ok, "empty command was accepted")
  assert_true(tostring(err):find("nonempty string", 1, true), "setup diagnostic")

  ok, err = pcall(env.plugin.setup, env.state, { command = false })
  assert_true(not ok, "non-string command was accepted")
  assert_true(tostring(err):find("nonempty string", 1, true), "setup type diagnostic")
end)

io.stdout:write(string.format("%d Yazi plugin tests passed\n", passed))

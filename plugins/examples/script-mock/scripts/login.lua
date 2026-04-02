function login(ctx, config, state)
    local password = secret.get("password")
    if password == nil or password == "" then
        return { ok = false, error = "missing password" }
    end

    secret.set("session_token", "session-" .. config.username)
    return {
        ok = true,
        state = {
            counter = 1
        }
    }
end

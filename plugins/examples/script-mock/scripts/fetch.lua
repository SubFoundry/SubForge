function fetch(ctx, config, state)
    if state == nil or (state.counter or 0) < 2 then
        return { ok = false, error = "invalid state for fetch" }
    end

    log.info("script-mock fetch subscription")
    return {
        ok = true,
        subscription = {
            url = config.subscription_url
        },
        state = {
            counter = (state.counter or 0) + 1
        }
    }
end

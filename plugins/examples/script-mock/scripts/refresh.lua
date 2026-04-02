function refresh(ctx, config, state)
    if state == nil then
        return { ok = false, error = "missing state" }
    end

    return {
        ok = true,
        state = {
            counter = (state.counter or 0) + 1
        }
    }
end

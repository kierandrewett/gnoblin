-- Reproduce the desktop's rounded windows and layered shadows in isolation.
return {
    ['window-rules'] = {
        {
            match = {type = 'window'},
            corners = {
                radius = 14,
                smoothing = 0.0,
                mode = 'force',
                shadow = {
                    {x = 0, y = 8, blur = 42, spread = 4, opacity = 0.22},
                    {x = 0, y = 3, blur = 14, spread = 1, opacity = 0.18},
                    {x = 0, y = 1, blur = 4, spread = 0, opacity = 0.22},
                },
            },
        },
        {
            match = {type = 'window'},
            borders = {
                ['inner-width'] = 1,
                ['inner-color'] = '#505050bf',
                ['outer-width'] = 1,
                ['outer-color'] = '#00000080',
                radius = 14,
                smoothing = 0.0,
            },
        },
        {
            match = {type = 'window', focused = true},
            corners = {shadow = {
                {x = 0, y = 18, blur = 64, spread = 8, opacity = 0.38},
                {x = 0, y = 8, blur = 24, spread = 2, opacity = 0.26},
                {x = 0, y = 2, blur = 6, spread = 0, opacity = 0.32},
            }},
        },
        {
            match = {type = 'window'},
            corners = {['shadow-animation'] = {duration = 180, easing = 'ease-out-cubic'}},
        },
    },
}

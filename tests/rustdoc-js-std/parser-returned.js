const PARSED = [
    {
        query: "-> F<P>",
        elems: [],
        foundElems: 1,
        original: "-> F<P>",
        returned: [{
            name: "f",
            fullPath: ["f"],
            pathWithoutLast: [],
            pathLast: "f",
            generics: [
                {
                    name: "p",
                    fullPath: ["p"],
                    pathWithoutLast: [],
                    pathLast: "p",
                    generics: [],
                },
            ],
            typeFilter: -1,
        }],
        userQuery: "-> f<p>",
        error: null,
    },
    {
        query: "-> P",
        elems: [],
        foundElems: 1,
        original: "-> P",
        returned: [{
            name: "p",
            fullPath: ["p"],
            pathWithoutLast: [],
            pathLast: "p",
            generics: [],
            typeFilter: -1,
        }],
        userQuery: "-> p",
        error: null,
    },
    {
        query: "->,a",
        elems: [],
        foundElems: 1,
        original: "->,a",
        returned: [{
            name: "a",
            fullPath: ["a"],
            pathWithoutLast: [],
            pathLast: "a",
            generics: [],
            typeFilter: -1,
        }],
        userQuery: "->,a",
        error: null,
    },
    {
        query: "aaaaa->a",
        elems: [{
            name: "aaaaa",
            fullPath: ["aaaaa"],
            pathWithoutLast: [],
            pathLast: "aaaaa",
            generics: [],
            typeFilter: -1,
        }],
        foundElems: 2,
        original: "aaaaa->a",
        returned: [{
            name: "a",
            fullPath: ["a"],
            pathWithoutLast: [],
            pathLast: "a",
            generics: [],
            typeFilter: -1,
        }],
        userQuery: "aaaaa->a",
        error: null,
    },
    {
        query: "-> !",
        elems: [],
        foundElems: 1,
        original: "-> !",
        returned: [{
            name: "never",
            fullPath: ["never"],
            pathWithoutLast: [],
            pathLast: "never",
            generics: [],
            typeFilter: 1,
        }],
        userQuery: "-> !",
        error: null,
    },
    {
        query: "a->",
        elems: [{
            name: "a",
            fullPath: ["a"],
            pathWithoutLast: [],
            pathLast: "a",
            generics: [],
            typeFilter: -1,
        }],
        foundElems: 1,
        original: "a->",
        returned: [],
        userQuery: "a->",
        hasReturnArrow: true,
        error: null,
    },
    {
        query: "!->",
        elems: [{
            name: "never",
            fullPath: ["never"],
            pathWithoutLast: [],
            pathLast: "never",
            generics: [],
            typeFilter: 1,
        }],
        foundElems: 1,
        original: "!->",
        returned: [],
        userQuery: "!->",
        hasReturnArrow: true,
        error: null,
    },
    {
        query: "! ->",
        elems: [{
            name: "never",
            fullPath: ["never"],
            pathWithoutLast: [],
            pathLast: "never",
            generics: [],
            typeFilter: 1,
        }],
        foundElems: 1,
        original: "! ->",
        returned: [],
        userQuery: "! ->",
        hasReturnArrow: true,
        error: null,
    },
    {
        query: "primitive:!->",
        elems: [{
            name: "never",
            fullPath: ["never"],
            pathWithoutLast: [],
            pathLast: "never",
            generics: [],
            typeFilter: 1,
        }],
        foundElems: 1,
        original: "primitive:!->",
        returned: [],
        userQuery: "primitive:!->",
        hasReturnArrow: true,
        error: null,
    },
];

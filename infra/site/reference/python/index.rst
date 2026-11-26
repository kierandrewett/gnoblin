Gnoblin Python SDK
==================

The ``gnoblin`` package that a Python shell imports: ``from gnoblin import gnoblin``. Every page is written from the
stubs that Gnoblin installs for editors and type checkers, and the descriptions are the comments of the schema files,
so this reference, the hover text in an editor and the other two languages say the same thing.

* ``gnoblin.objects.model`` is the object model of the compositor: windows, outputs, input, the session and processes.
* ``gnoblin.objects.std`` is the desktop above it: workspaces, layouts, the scene, styles and widgets.
* ``gnoblin`` and ``gnoblin.std`` are the first generation of the SDK: named subscriptions and the tree of nodes.

The same reference exists for `TypeScript </reference/typescript/>`_ and `Rust </reference/rust/>`_, and the
`guide to Python shells </docs/python-shell>`_ explains how to use it.

.. toctree::
   :maxdepth: 2
   :caption: Reference

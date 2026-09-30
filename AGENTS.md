这是一个试图用GPUI来复刻网易云音乐的项目，是一个练手项目，因此应该最大化保持用户自主编写。在用户没有明确说明的情况下，你不需要大幅地跨文件实现新功能，仅应该辅助用户查阅文档及做错误修正，或者向用户提供接下来如何编写的帮助。

GPUI offers three different registers depending on your needs:

State management and communication with Entity's. Whenever you need to store application state that communicates between different parts of your application, you'll want to use GPUI's entities. Entities are owned by GPUI and are only accessible through an owned smart pointer similar to an Rc. See the app::context module for more information.

High level, declarative UI with views. All UI in GPUI starts with a view. A view is simply an Entity that can be rendered, by implementing the Render trait. At the start of each frame, GPUI will call this render method on the root view of a given window. Views build a tree of elements, lay them out and style them with a tailwind-style API, and then give them to GPUI to turn into pixels. See the div element for an all purpose swiss-army knife of rendering.

Low level, imperative UI with Elements. Elements are the building blocks of UI in GPUI, and they provide a nice wrapper around an imperative API that provides as much flexibility and control as you need. Elements have total control over how they and their child elements are rendered and can be used for making efficient views into large lists, implement custom layouting for a code editor, and anything else you can think of. See the element module for more information.

Each of these registers has one or more corresponding contexts that can be accessed from all GPUI services. This context is your main interface to GPUI, and is used extensively throughout the framework.
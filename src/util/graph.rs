//! # Graph Utility
//!
//! The following are ramblings I wanted to capture on the thought process behind why this utility exists.
//!
//! Part of it is barganing with myself on adding a new abstraction and justifying my resistance to normal patterns.
//!
//! ## The Problem
//!
//! Today, Peek is a very useful utility. We can essentially read fields from objects without allocating memory to hold the value of that field.
//!
//! The issue is that in order to achieve this we create a flexbuffer to serialize this data. A flexbuffer root cannot mutate after it is created.
//!
//! This creates a composability problem for objects we define but more importantly how we write code for those objects.
//!
//! ## Traditional OOP
//!
//! So say we're a hardware company and we want to define objects to represent our products. Let's say we make phones.
//!
//! ```
//! Phone { ... }
//! ```
//!
//! So in order to build this phone we need components, probably like metal, electronics, software, etc. So lets add those components as fields.
//!
//! ```
//! Phone {
//!   metal: Metal,
//!   electronics: Electronics,
//!   software: Software
//! }
//! ```
//!
//! So great, but now my hardware company starts to make a bunch of other things, computers, tablets, etc. Those new things are made up of the same components.
//!
//! ```
//! Phone {
//!   metal: Metal,
//!   electronics: Electronics,
//!   software: Software
//! }
//!
//! Computer {
//!   metal: Metal,
//!   electronics: Electronics,
//!   software: Software
//! }
//!
//! Tablet {
//!   metal: Metal,
//!   electronics: Electronics,
//!   software: Software
//! }
//! ```
//!
//! So traditional OOP would say, hey these three things look alike, so how about we define a base class to use so that we can share all of the code between these things.
//!
//! ```
//! Hardware {
//!   metal: Metal,
//!   electronics: Electronics,
//!   software: Software
//! }
//!
//! Phone(Hardware)
//!
//! Computer(Hardware)
//!
//! Tablet(Hardware)
//! ```
//!
//! So that works great for these three things, but say we want to diversify what we make?
//!
//! ```
//! ArtificialHeart(Hardware)
//! ```
//!
//! So maybe we don't use metal in this though, so now we need to either define a new base-class `MedicalHardware`, or refactor `Metal`.
//!
//! Without going through the entire process, in the real-world this balloons and just adds layers of tech-debt and subtle bugs, and harder to read code.
//!
//! I noticed one of the biggest issues switching from a traditional OOP language to Rust was the urge to model my types this way, and while it works
//! for most cases, sometimes it feels more bloated then it needs to be.
//!
//! ## Traditional OOP vs Flexbuffers/Peek
//!
//! So now the immediate issue is that, this type of OOP and flexbuffers do not play very well together. So take a step back, I wanted to think about what I'm actually
//! trying to achieve here. The underlying thing of course is, modularity, components. We all like the idea of components we can just swap in and out in order to compose a thing.
//!
//! If we try to cram what we would do in traditional OOP into flexbuffer roots, we would end up either putting all of our code in the first type to define the component, or needing to redefine
//! our root object whenever we wanted to modify the component. This might be okay for one object, but in the example I gave above, it would mean needing to duplicate that code across multiple
//! different types of products.
//!
//! What I really want is to take Metal and have it be it's own thing. Changing the properties of Metal should be independent of Phone, at least in the flexbuffer sense.
//!
//! ## A solution
//!
//! So it has taken a while for my brain to come up with a strategy that addresses the issue and my resistance to use a traditional pattern I'm used to.
//!
//! Of course all of these patterns have probably already been defined, but a lot of the times they are designed without real-world constraints.
//!
//! In my head this is what would happen if I did it the traditional way,
//!
//! ```
//! Phone {
//!   metal: Metal,
//!   ..
//! }
//!
//! Computer {
//!   metal: Metal,
//!  ..
//! }
//! ```
//!
//! Say these two things used the same metal. If I were to turn this into a flexbuffer, the fields for metal would live in two different buffers. This is virtually a copy.
//!
//! What I really want is a single definition of Metal, and for both Phone and Computer to reference that. But flexbuffers doesn't have a reference type.
//!
//! So this is when I started to think of the "Graph" utility module.
//!
//! The high level idea is that you would be able to do something like this:
//!
//! ```
//! let phone: Phone;
//!
//! let metal = phone.at("metal");
//! ..
//! ```
//!
//! You'd re-use the Peek interface, and I would somehow manage the details underneath the hood. I gave this a thought and in my head it ballooned the complexity of what Peek does by quite a bit.
//!
//! So then I realized I needed a new abstraction. However, the issue I had was, what should the api or language look like?
//!
//! ## `Graph` api
//!
//! So first off, maybe it's not perfect, but I settled on the keyword `of`.
//!
//! ```
//! let phone: Phone;
//!
//! if let Some(metal) = phone.of("metal") {
//!   ..
//! }
//! ```
//!
//! So this roughly translates into, if a phone is a composite with a component called "metal", then return a peek interface for that component.
//!
//! I'm handwaving some details here, but I thought this was a powerful distinction.
//!
//! It solves the issues I have with Traditional OOP. I don't have to worry about the cost of future OOP code and abstractions in case I have similar types.
//! I can just define new components as I like, and be able to add them to my types as I wish. It lowers the abstraction barrier, and keeps the code close to the data.
//!
//! Now the issue is, how do I build this graph?
//!
//! There are several ways I could approach this and it might end up being the case of multiple entrypoints.
//!
//! First, of all the natural choice for me would be to automatically compose this from a Container Record. The layers are inherently components. However, what if I wanted to go the other direction?
//!
//! What if I wanted to start from a record, create a graph, and commit that as a Container? That should be possible too.
//!
//! ## Theory-crafting
//!
//! So in the first stage, I need the mechanics of what I want to happen. A Graph structure is probably the first step.
//!
//! Even if it's just a map of names to Peek structs, that's actually good enough for now to solve the issue.
//!
//! The next stage is how this gets composed. It's pretty straight-forward. This is essentially just a traditional map, so really it just needs the normal insert/remove functions.
//!
//! Last stage is being able to do this across a wide range of peek situations.
//!

use serde::Deserialize;

use crate::util::{Intern, Peek, PeekExtensions};
use std::sync::Arc;

#[derive(Clone)]
pub enum Node<'peek> {
    Single(Peek<'peek>),
    Composite(Arc<Graph<'peek>>),
}

/// Graph is a thread-safe composite view of multiple Peek references
pub struct Graph<'peek> {
    root: Node<'peek>,
    map: Arc<dashmap::DashMap<&'static str, Node<'peek>>>,
}

impl<'peek> Graph<'peek> {
    /// Inserts a component into the graph
    ///
    /// Returns the previous component if one already existed
    #[inline]
    pub fn insert(&self, name: &str, component: Node<'peek>) -> Option<Node<'peek>> {
        self.map.insert(name.intern(), component)
    }

    /// Returns the number of components in the graph
    #[inline]
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Returns true if the graph is empty
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Converts the graph into a node
    #[inline]
    pub fn to_node(self) -> Node<'peek> {
        Node::Composite(Arc::new(self))
    }
}

impl<'peek> From<Peek<'peek>> for Graph<'peek> {
    fn from(value: Peek<'peek>) -> Self {
        Self {
            root: Node::Single(value),
            map: Arc::default(),
        }
    }
}

impl<'peek> From<Arc<Graph<'peek>>> for Graph<'peek> {
    fn from(value: Arc<Graph<'peek>>) -> Self {
        Self {
            root: Node::Composite(value),
            map: Arc::default(),
        }
    }
}

impl<'peek> From<Node<'peek>> for Graph<'peek> {
    fn from(value: Node<'peek>) -> Self {
        Self {
            root: value,
            map: Arc::default(),
        }
    }
}

impl<'peek> Clone for Graph<'peek> {
    fn clone(&self) -> Self {
        Self {
            root: self.root.clone(),
            map: self.map.clone(),
        }
    }
}

/// Trait for types that can project from a Peek
pub trait Component<'peek>: Deserialize<'peek> + Sized {
    const SYMBOL: &'static str;
}

/// API for working with graph model
pub trait GraphExtensions<'peek> {
    /// Returns a component from the graph
    fn of<C: Component<'peek>>(&self) -> Option<C> {
        self.of_ty(C::SYMBOL)
            .and_then(|c| c.val().and_then(|v| v.to_obj::<C>()))
    }

    /// Returns a component from the graph as type `C`
    fn of_as<C: Component<'peek>>(&self, name: &str) -> Option<C> {
        self.of_ty(name).and_then(|v| v.to_obj::<C>())
    }

    /// Returns a component from the graph by name
    fn of_ty(&self, name: &str) -> Option<Node<'peek>>;
}

impl<'peek> GraphExtensions<'peek> for Graph<'peek> {
    fn of_ty(&self, name: &str) -> Option<Node<'peek>> {
        self.map.get(name).map(|v| v.clone())
    }
}

impl<'peek> GraphExtensions<'peek> for Node<'peek> {
    fn of_ty(&self, name: &str) -> Option<Node<'peek>> {
        match self {
            Node::Single(..) => None,
            Node::Composite(graph) => graph.of_ty(name),
        }
    }
}

impl<'peek> GraphExtensions<'peek> for Option<Node<'peek>> {
    fn of_ty(&self, name: &str) -> Option<Node<'peek>> {
        self.as_ref().and_then(|v| match v {
            Node::Single(..) => None,
            Node::Composite(graph) => graph.of_ty(name),
        })
    }
}

impl<'peek> From<Peek<'peek>> for Node<'peek> {
    fn from(value: Peek<'peek>) -> Self {
        Self::Single(value)
    }
}

impl<'peek> PeekExtensions<'peek> for Node<'peek> {
    fn in_ref(
        self,
    ) -> impl std::ops::Index<&'peek str, Output = super::peek::PeekRef<'peek>>
    + super::PeekRefExtensions<'peek> {
        match self {
            Node::Single(peek) => peek.in_ref(),
            Node::Composite(graph) => graph.root.clone().in_ref(),
        }
    }

    fn at(self, key: &str) -> Option<Peek<'peek>> {
        match self {
            Node::Single(peek) => peek.at(key),
            Node::Composite(graph) => graph.root.clone().at(key),
        }
    }

    fn at_path(self, keys: impl AsRef<[&'peek str]>) -> Option<Peek<'peek>> {
        match self {
            Node::Single(peek) => peek.at_path(keys),
            Node::Composite(graph) => graph.root.clone().at_path(keys),
        }
    }

    fn at_dot(self, path: &'peek str) -> Option<Peek<'peek>> {
        match self {
            Node::Single(peek) => peek.at_dot(path),
            Node::Composite(graph) => graph.root.clone().at_dot(path),
        }
    }

    fn bool(self) -> Option<bool> {
        match self {
            Node::Single(peek) => peek.bool(),
            Node::Composite(graph) => graph.root.clone().bool(),
        }
    }

    fn str(self) -> Option<&'peek str> {
        match self {
            Node::Single(peek) => peek.str(),
            Node::Composite(graph) => graph.root.clone().str(),
        }
    }

    fn u64(self) -> Option<u64> {
        match self {
            Node::Single(peek) => peek.u64(),
            Node::Composite(graph) => graph.root.clone().u64(),
        }
    }

    fn int(self) -> Option<i64> {
        match self {
            Node::Single(peek) => peek.int(),
            Node::Composite(graph) => graph.root.clone().int(),
        }
    }

    fn blob(self) -> Option<&'peek [u8]> {
        match self {
            Node::Single(peek) => peek.blob(),
            Node::Composite(graph) => graph.root.clone().blob(),
        }
    }

    fn iter(self) -> Option<impl Iterator<Item = Peek<'peek>>> {
        match self {
            Node::Single(peek) => peek.iter(),
            Node::Composite(graph) => graph.root.clone().iter(),
        }
    }

    fn iter_kv(self) -> Option<impl Iterator<Item = (&'peek str, Peek<'peek>)>> {
        match self {
            Node::Single(peek) => peek.iter_kv(),
            Node::Composite(graph) => graph.root.clone().iter_kv(),
        }
    }

    fn val(self) -> Option<Peek<'peek>> {
        match self {
            Node::Single(peek) => peek.val(),
            Node::Composite(graph) => graph.root.clone().val(),
        }
    }
}

impl<'peek> PeekExtensions<'peek> for Option<Node<'peek>> {
    fn in_ref(
        self,
    ) -> impl std::ops::Index<&'peek str, Output = super::peek::PeekRef<'peek>>
    + super::PeekRefExtensions<'peek> {
        self.val().in_ref()
    }

    fn at(self, key: &str) -> Option<Peek<'peek>> {
        self.and_then(|s| s.at(key))
    }

    fn at_path(self, keys: impl AsRef<[&'peek str]>) -> Option<Peek<'peek>> {
        self.and_then(|s| s.at_path(keys))
    }

    fn at_dot(self, path: &'peek str) -> Option<Peek<'peek>> {
        self.and_then(|s| s.at_dot(path))
    }

    fn bool(self) -> Option<bool> {
        self.and_then(|s| s.bool())
    }

    fn str(self) -> Option<&'peek str> {
        self.and_then(|s| s.str())
    }

    fn u64(self) -> Option<u64> {
        self.and_then(|s| s.u64())
    }

    fn int(self) -> Option<i64> {
        self.and_then(|s| s.int())
    }

    fn blob(self) -> Option<&'peek [u8]> {
        self.and_then(|s| s.blob())
    }

    fn iter(self) -> Option<impl Iterator<Item = Peek<'peek>>> {
        self.and_then(|s| s.iter())
    }

    fn iter_kv(self) -> Option<impl Iterator<Item = (&'peek str, Peek<'peek>)>> {
        self.and_then(|s| s.iter_kv())
    }

    fn val(self) -> Option<Peek<'peek>> {
        self.and_then(|s| s.val())
    }
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use crate::{
        IRecord, Namespace,
        util::{Graph, GraphExtensions, PeekExtensions, graph::Component},
    };

    #[test]
    fn test_graph_api() {
        let graph = Namespace::ephemeral().author("example", |mut b| {
            b.start_map().push("name", "hello world");
            b
        });

        let component = Namespace::ephemeral().author("component", |mut b| {
            b.start_map().push("value", "cool component");
            b
        });

        let component2 = Namespace::ephemeral().author("component2", |mut b| {
            b.start_map().push("value", "cool component");
            b
        });

        let graph: Graph = graph.peek().val().unwrap().into();
        graph.insert("component1", component.peek().val().unwrap().into());
        graph.insert("component2", component2.peek().val().unwrap().into());

        assert_eq!(graph.of::<Component1>().unwrap().value, "cool component");
        assert_eq!(
            graph.of_ty("component1").at("value").str().unwrap(),
            "cool component"
        );

        // Try duck-typing
        assert_eq!(
            graph.of_as::<Component1>("component2").unwrap().value,
            "cool component"
        );
        assert_eq!(
            graph
                .to_node()
                .of_ty("component1")
                .at("value")
                .str()
                .unwrap(),
            "cool component"
        );
    }

    #[test]
    fn test_graph_transumtation() {
        let graph = Namespace::ephemeral().author("example", |mut b| {
            b.start_map().push("name", "hello world");
            b
        });

        let component = Namespace::ephemeral().author("component", |mut b| {
            b.start_map().push("value", "cool component");
            b
        });

        let component2 = Namespace::ephemeral().author("component2", |mut b| {
            b.start_map().push("value", "cool component");
            b
        });

        let graph = graph.peek().to_graph().unwrap();
        graph.insert("component1", component.peek().to_node().unwrap());
        graph.insert("component2", component2.peek().to_node().unwrap());

        assert_eq!(graph.of::<Component1>().unwrap().value, "cool component");
        assert_eq!(
            graph.of_ty("component1").at("value").str().unwrap(),
            "cool component"
        );

        // Try duck-typing
        assert_eq!(
            graph.of_as::<Component1>("component2").unwrap().value,
            "cool component"
        );
        assert_eq!(
            graph
                .to_node()
                .of_ty("component1")
                .at("value")
                .str()
                .unwrap(),
            "cool component"
        );
    }

    #[derive(Deserialize)]
    struct Component1<'peek> {
        value: &'peek str,
    }

    impl<'peek> Component<'peek> for Component1<'peek> {
        const SYMBOL: &'static str = "component1";
    }
}

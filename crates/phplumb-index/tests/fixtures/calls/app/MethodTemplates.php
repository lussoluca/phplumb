<?php

namespace App;

class Post
{
    public function /*@target:bound*/title(): string
    {
        return '';
    }
}

class Comment
{
    public function /*@target:shadow*/body(): string
    {
        return '';
    }
}

/** @template T */
class Repository
{
    /** @return list<T> */
    public function findBy(): array
    {
        return [];
    }
}

class Manager
{
    /**
     * @template T
     * @param class-string<T> $class
     * @return Repository<T>
     */
    public function getRepository(string $class): Repository
    {
        return new Repository();
    }
}

/** @template T */
class Box
{
    /**
     * @template T
     * @param class-string<T> $class
     * @return T
     */
    public function make(string $class)
    {
        return new $class();
    }
}

class Templated
{
    /** @param Box<Post> $box */
    public function run(Manager $manager, Box $box, string $name): void
    {
        $manager->getRepository(Post::class)->findBy()[0]->/*@cursor:bound*/title();
        $box->make(Comment::class)->/*@cursor:shadow*/body();
        $manager->getRepository($name)->findBy()[0]->/*@cursor:unbound*/title();
    }
}

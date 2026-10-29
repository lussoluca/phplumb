<?php

namespace App\Controller;

use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\Routing\Generator\UrlGeneratorInterface;

class LinksController extends AbstractController
{
    public function __construct(private UrlGeneratorInterface $urls)
    {
    }

    public function links(string $name): void
    {
        $this->redirectToRoute('home');
        $this->generateUrl('admin_users');
        $this->urls->generate('app_blog_show');
        $this->redirectToRoute('missing');
        $this->redirectToRoute($name);
        $this->redirectToRoute(route: 'home');
        $this->urls->generate(parameters: [], name: 'about');
        $this->redirectToRoute('home', status: 'about');
    }
}
